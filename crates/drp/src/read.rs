//! Reads a Resolve project (.drp) or timeline (.drt) export without Resolve. Both are a ZIP of XML:
//! project.xml, MediaPool/**/MpFolder.xml (one per bin), SeqContainer/<uuid>.xml (one per timeline or
//! compound clip) and Gallery.xml. Objects are `<Type DbId="uuid">` and refer to each other by DbId.

use std::collections::HashMap;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use serde::Serialize;

use crate::blobs::{
    KeyedValue, clip_path, decode_in, extents, frame_rate, from_hex, read_keyed_dict, resolution,
    version_stamp,
};
use crate::xml::{XmlElement, child, child_text, descendants, parse_xml, text_of};
use crate::zip::{Zip, entry_data, read_zip};
use crate::{Result, fail};

#[derive(Debug, Clone)]
pub struct DrpFile {
    pub zip: Zip,
    /// Every XML entry's text, by entry name, in the ZIP's order.
    pub xml: IndexMap<String, String>,
    pub project: Project,
    /// The top-level bins, normally just Master.
    pub media_pool: Vec<MediaFolder>,
    /// Timelines and compound clips, each with its tracks.
    pub timelines: Vec<Timeline>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    /// `<ProjectName>`; a .drt carries the timeline's name here.
    pub name: String,
    /// From the version comment: the Resolve that wrote the file, like "21.0.4.0005", and its project format.
    pub db_app_ver: Option<String>,
    pub db_prj_ver: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaFolder {
    pub id: String,
    pub name: String,
    pub folders: Vec<MediaFolder>,
    pub items: Vec<MediaItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MediaItem {
    pub id: String,
    /// From the element name: "video", "audio", "timeline", "compound", or the raw name of anything else.
    pub kind: String,
    pub name: String,
    /// The file Resolve links the item to, from its `<Clip>` blob.
    pub path: Option<String>,
    pub fps: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Length in frames at `fps`.
    pub frames: Option<i64>,
    /// A timeline or compound clip's first frame.
    pub start: Option<i64>,
    /// The `Sm2Sequence` of a timeline or compound clip, which its tracks point at.
    pub sequence_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Timeline {
    /// The media pool item: a timeline or a compound clip.
    pub id: String,
    pub kind: String,
    pub name: String,
    /// The SeqContainer entry holding the tracks, and its DbId.
    pub entry: String,
    pub container_id: String,
    pub sequence_id: Option<String>,
    pub fps: Option<f64>,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// First frame, like 86400 for 01:00:00:00 at 24 fps, and length in frames.
    pub start: Option<i64>,
    pub frames: Option<i64>,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackType {
    Video,
    Audio,
    Subtitle,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Track {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: TrackType,
    /// 1-based among tracks of the same type, like V1 and A2.
    pub index: usize,
    pub name: String,
    pub clips: Vec<TimelineClip>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TimelineClip {
    pub id: String,
    /// From the element name: "video", "audio", or the raw name of anything else on a track.
    pub kind: String,
    pub name: String,
    /// Record in and out, in timeline frames; out is exclusive.
    pub start: i64,
    pub end: i64,
    /// Source in and out in timeline frames, from `<In>`; out assumes the clip plays at 100%.
    pub source_in: Option<i64>,
    pub source_out: Option<i64>,
    /// The media pool item the clip plays.
    pub media_id: Option<String>,
    /// The plain-text path on the clip. Resolve relinks by the media pool item's path, not this.
    pub media_path: Option<String>,
    pub media_fps: Option<f64>,
    /// Whether the clip is enabled. Always None for now: no sample file has a disabled clip, so where Resolve
    /// stores the flag isn't known yet.
    pub enabled: Option<bool>,
}

pub(crate) static SEQ_CONTAINER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(^|/)SeqContainer(/[^/]+|[0-9]*)\.xml$").unwrap());
pub(crate) static MP_FOLDER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^MediaPool/(.+/)?MpFolder\.xml$").unwrap());
static MP_KIND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Sm2Mp([0-9A-Za-z_]+?)Clip$").unwrap());
static TI_KIND: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^Sm2Ti([0-9A-Za-z_]+?)Clip$").unwrap());

/// Reads a .drp or .drt file's contents. Errs when it isn't one.
pub fn read_drp(buffer: &[u8]) -> Result<DrpFile> {
    let zip = read_zip(buffer)?;
    let mut xml = IndexMap::new();
    for entry in &zip.entries {
        if entry.name.ends_with(".xml") {
            xml.insert(
                entry.name.clone(),
                String::from_utf8_lossy(&entry_data(entry)?).into_owned(),
            );
        }
    }
    if !xml.contains_key("project.xml") && !xml.keys().any(|name| SEQ_CONTAINER.is_match(name)) {
        return fail("Not a Resolve project or timeline: it has no project.xml or timelines.");
    }

    let project_xml = xml.get("project.xml");
    let version = project_xml
        .and_then(|text| version_stamp(text))
        .or_else(|| xml.values().find_map(|text| version_stamp(text)));
    let name = match project_xml {
        Some(text) => child_text(text, &parse_xml(text)?, "ProjectName"),
        None => String::new(),
    };

    let media_pool = read_media_pool(&xml)?;
    let mut by_sequence = HashMap::new();
    for item in media_items(&media_pool) {
        if let Some(sequence) = &item.sequence_id {
            by_sequence.insert(sequence.clone(), item);
        }
    }

    let mut timelines = Vec::new();
    for (entry, text) in &xml {
        if SEQ_CONTAINER.is_match(entry) {
            timelines.push(read_timeline(entry, text, &by_sequence)?);
        }
    }
    let project = Project {
        name,
        db_app_ver: version.as_ref().map(|stamp| stamp.db_app_ver.clone()),
        db_prj_ver: version.as_ref().map(|stamp| stamp.db_prj_ver),
    };
    Ok(DrpFile {
        zip,
        xml,
        project,
        media_pool,
        timelines,
    })
}

/// Every item in the media pool, depth first.
pub fn media_items(folders: &[MediaFolder]) -> Vec<&MediaItem> {
    let mut all = Vec::new();
    for folder in folders {
        all.extend(&folder.items);
        all.extend(media_items(&folder.folders));
    }
    all
}

// Each bin is its own MpFolder.xml, naming its parent bin's DbId in <MpFolder>.
fn read_media_pool(xml: &IndexMap<String, String>) -> Result<Vec<MediaFolder>> {
    let mut folders = Vec::new();
    let mut parents = Vec::new();
    for (entry, text) in xml {
        if !MP_FOLDER.is_match(entry) {
            continue;
        }
        let root = parse_xml(text)?;
        let items = child(&root, "MediaVec")
            .map(|vec| {
                vec.children
                    .iter()
                    .flat_map(|element| &element.children)
                    .map(|item| read_media_item(text, item))
                    .collect()
            })
            .unwrap_or_default();
        folders.push(Some(MediaFolder {
            id: root.id.clone().unwrap_or_default(),
            name: child_text(text, &root, "Name"),
            folders: Vec::new(),
            items,
        }));
        parents.push(child_text(text, &root, "MpFolder"));
    }
    // The last bin with an id wins, like a Map built in order.
    let by_id: HashMap<String, usize> = folders
        .iter()
        .enumerate()
        .map(|(i, folder)| (folder.as_ref().expect("all set").id.clone(), i))
        .collect();
    let mut below: Vec<Vec<usize>> = vec![Vec::new(); folders.len()];
    let mut top = Vec::new();
    for (i, parent) in parents.iter().enumerate() {
        match by_id.get(parent) {
            Some(&above) if above != i => below[above].push(i),
            _ => top.push(i),
        }
    }
    fn build(i: usize, folders: &mut [Option<MediaFolder>], below: &[Vec<usize>]) -> MediaFolder {
        let mut folder = folders[i].take().expect("each bin has one place");
        folder.folders = below[i]
            .iter()
            .map(|&next| build(next, folders, below))
            .collect();
        folder
    }
    Ok(top
        .into_iter()
        .map(|i| build(i, &mut folders, &below))
        .collect())
}

fn read_media_item(text: &str, element: &XmlElement) -> MediaItem {
    let mut item = MediaItem {
        id: element.id.clone().unwrap_or_default(),
        kind: kind(&MP_KIND, &element.name),
        name: child_text(text, element, "Name"),
        path: None,
        fps: None,
        width: None,
        height: None,
        frames: None,
        start: None,
        sequence_id: None,
    };
    for node in descendants(element) {
        if node.name == "Clip" && item.path.is_none() {
            item.path = clip_path(&text_of(text, Some(node))).ok();
        } else if node.name == "BtVideoInfo" {
            let time = read_keyed_dict(&child_text(text, node, "Time")).ok();
            let geometry = read_keyed_dict(&child_text(text, node, "Geometry")).ok();
            match time.as_ref().and_then(|time| time.get("NumFrames")) {
                Some(KeyedValue::Int32(frames)) => item.frames = Some(i64::from(*frames)),
                Some(KeyedValue::UInt32(frames)) => item.frames = Some(i64::from(*frames)),
                Some(KeyedValue::Int64(frames)) => item.frames = Some(*frames),
                Some(KeyedValue::Double(frames)) => item.frames = Some(*frames as i64),
                _ => {}
            }
            if let Some(KeyedValue::Bytes(rate)) =
                time.as_ref().and_then(|time| time.get("FrameRate"))
            {
                item.fps = frame_rate(rate);
            }
            if let Some(KeyedValue::Bytes(size)) = geometry
                .as_ref()
                .and_then(|geometry| geometry.get("Resolution"))
                && let Some((width, height)) = resolution(size)
            {
                (item.width, item.height) = (Some(width), Some(height));
            }
        } else if node.name == "Sm2Sequence" && item.sequence_id.is_none() {
            item.sequence_id = node.id.clone();
            item.fps = frame_rate(&hex(text, node, "FrameRate"));
            if let Some((width, height)) = resolution(&hex(text, node, "Resolution")) {
                (item.width, item.height) = (Some(width), Some(height));
            }
            if let (Some((start, duration)), Some(fps)) =
                (extents(&hex(text, node, "MediaExtents")), item.fps)
                && fps != 0.0
                && !fps.is_nan()
            {
                item.start = Some(js_round(start * fps));
                item.frames = Some(js_round(duration * fps));
            }
        }
    }
    item
}

fn read_timeline(
    entry: &str,
    text: &str,
    by_sequence: &HashMap<String, &MediaItem>,
) -> Result<Timeline> {
    let root = parse_xml(text)?;
    let mut tracks = Vec::new();
    let mut counts: HashMap<TrackType, usize> = HashMap::new();
    let elements = track_elements(&root);
    for track in &elements {
        let number = js_number(&child_text(text, track, "Type"));
        let kind = match number {
            0.0 => TrackType::Video,
            1.0 => TrackType::Audio,
            2.0 => TrackType::Subtitle,
            _ => TrackType::Other,
        };
        let count = counts.entry(kind).or_default();
        *count += 1;
        tracks.push(Track {
            id: track.id.clone().unwrap_or_default(),
            kind,
            index: *count,
            name: child_text(text, track, "UserDefinedName"),
            clips: clip_elements(track)
                .into_iter()
                .map(|clip| read_clip(text, clip))
                .collect(),
        });
    }
    let sequence_id = elements
        .first()
        .map(|track| child_text(text, track, "Sequence"))
        .filter(|id| !id.is_empty());
    let item = sequence_id.as_ref().and_then(|id| by_sequence.get(id));
    Ok(Timeline {
        id: item
            .map(|item| item.id.clone())
            .or_else(|| root.id.clone())
            .unwrap_or_default(),
        kind: item.map_or("timeline".into(), |item| item.kind.clone()),
        name: item.map(|item| item.name.clone()).unwrap_or_default(),
        entry: entry.to_string(),
        container_id: root.id.clone().unwrap_or_default(),
        sequence_id,
        fps: item.and_then(|item| item.fps),
        width: item.and_then(|item| item.width),
        height: item.and_then(|item| item.height),
        start: item.and_then(|item| item.start),
        frames: item.and_then(|item| item.frames),
        tracks,
    })
}

/// A SeqContainer's `Sm2TiTrack`s, in the order of its `…TrackVec`s.
pub(crate) fn track_elements(root: &XmlElement) -> Vec<&XmlElement> {
    root.children
        .iter()
        .filter(|element| element.name.ends_with("TrackVec"))
        .flat_map(|vec| &vec.children)
        .filter_map(|element| child(element, "Sm2TiTrack"))
        .collect()
}

/// A track's clips: what sits in each `<Element>` of its `<Items>`.
pub(crate) fn clip_elements(track: &XmlElement) -> Vec<&XmlElement> {
    child(track, "Items")
        .map(|items| {
            items
                .children
                .iter()
                .flat_map(|element| &element.children)
                .collect()
        })
        .unwrap_or_default()
}

fn read_clip(text: &str, element: &XmlElement) -> TimelineClip {
    let start = frames_of(&child_text(text, element, "Start"));
    let end = start + frames_of(&child_text(text, element, "Duration"));
    let source_in = decode_in(&child_text(text, element, "In")).ok();
    let not_empty = |value: String| Some(value).filter(|value| !value.is_empty());
    TimelineClip {
        id: element.id.clone().unwrap_or_default(),
        kind: kind(&TI_KIND, &element.name),
        name: child_text(text, element, "Name"),
        start,
        end,
        source_in,
        source_out: source_in.map(|source_in| source_in + end - start),
        media_id: not_empty(child_text(text, element, "MediaRef")),
        media_path: not_empty(child_text(text, element, "MediaFilePath")),
        media_fps: frame_rate(&hex(text, element, "MediaFrameRate")),
        enabled: None,
    }
}

fn kind(pattern: &Regex, name: &str) -> String {
    pattern
        .captures(name)
        .map_or_else(|| name.to_string(), |captures| captures[1].to_lowercase())
}

fn hex(text: &str, element: &XmlElement, name: &str) -> Vec<u8> {
    from_hex(&child_text(text, element, name))
}

/// JavaScript's `Number(text)` for the decimal numbers Resolve writes: "" is 0, anything else unreadable NaN.
pub(crate) fn js_number(text: &str) -> f64 {
    let text = text.trim();
    if text.is_empty() {
        return 0.0;
    }
    match text.strip_prefix(['+', '-']).unwrap_or(text) {
        "Infinity" => {
            return if text.starts_with('-') {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            };
        }
        rest if rest
            .chars()
            .any(|c| c.is_alphabetic() && c != 'e' && c != 'E') =>
        {
            return f64::NAN;
        }
        _ => {}
    }
    text.parse().unwrap_or(f64::NAN)
}

/// A frame count from XML text; what isn't a number reads as 0.
pub(crate) fn frames_of(text: &str) -> i64 {
    let number = js_number(text);
    if number.is_finite() { number as i64 } else { 0 }
}

/// JavaScript's `Math.round`: halves round up, not away from zero.
pub(crate) fn js_round(value: f64) -> i64 {
    let floor = value.floor();
    (if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }) as i64
}
