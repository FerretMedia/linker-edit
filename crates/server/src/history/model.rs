// The on-disk history format: one folder per timeline, split by domain like Vit
// (github.com/LucasHJin/vit) so separate people's cut, audio and marker changes merge cleanly.
//
// The files are written with crate::js::stringify, so they are byte for byte what the TypeScript server wrote:
// both apps save into the same history.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Map, Value};

use crate::js;

/// A field that may be left out (None) or null (Some(Value::Null)): Number() reads the two differently.
fn defined<'de, D: Deserializer<'de>>(deserializer: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

/// What snapshot.py prints.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RawSnapshot {
    pub project: String,
    /// Resolve's unique id for the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    #[serde(default)]
    pub current_timeline: Option<String>,
    #[serde(default)]
    pub timelines: Vec<RawTimeline>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawTimeline {
    pub id: String,
    pub name: String,
    #[serde(
        default,
        deserialize_with = "defined",
        skip_serializing_if = "Option::is_none"
    )]
    pub frame_rate: Option<Value>,
    #[serde(
        default,
        deserialize_with = "defined",
        skip_serializing_if = "Option::is_none"
    )]
    pub width: Option<Value>,
    #[serde(
        default,
        deserialize_with = "defined",
        skip_serializing_if = "Option::is_none"
    )]
    pub height: Option<Value>,
    #[serde(
        default,
        deserialize_with = "defined",
        skip_serializing_if = "Option::is_none"
    )]
    pub start_frame: Option<Value>,
    #[serde(
        default,
        deserialize_with = "defined",
        skip_serializing_if = "Option::is_none"
    )]
    pub end_frame: Option<Value>,
    #[serde(default)]
    pub start_timecode: Option<String>,
    #[serde(default)]
    pub video: Vec<RawTrack>,
    #[serde(default)]
    pub audio: Vec<RawTrack>,
    /// `{ frame: number } & Record<string, unknown>`.
    #[serde(default)]
    pub markers: Vec<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawTrack {
    pub index: f64,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub items: Vec<RawItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawItem {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "defined",
        skip_serializing_if = "Option::is_none"
    )]
    pub start: Option<Value>,
    #[serde(
        default,
        deserialize_with = "defined",
        skip_serializing_if = "Option::is_none"
    )]
    pub end: Option<Value>,
    #[serde(default)]
    pub source_start: Option<f64>,
    #[serde(default)]
    pub source_end: Option<f64>,
    #[serde(default)]
    pub media: Option<RawMedia>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub properties: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fusion_comps: Option<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawMedia {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    pub path: String,
    #[serde(
        default,
        deserialize_with = "defined",
        skip_serializing_if = "Option::is_none"
    )]
    pub fps: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Clip {
    pub track: f64,
    pub id: String,
    pub name: String,
    pub start: f64,
    pub end: f64,
    #[serde(default)]
    pub source_start: Option<f64>,
    #[serde(default)]
    pub source_end: Option<f64>,
    /// Media pool item id; null for generators and titles.
    #[serde(default)]
    pub media: Option<String>,
    /// Only ever false; left out when the clip is enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fusion_comps: Option<f64>,
    /// Only values that differ from Resolve's defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub properties: Option<Map<String, Value>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Marker {
    pub frame: f64,
    pub color: String,
    pub name: String,
    pub note: String,
    pub duration: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TimelineMetadata {
    pub id: String,
    pub name: String,
    pub frame_rate: f64,
    pub width: f64,
    pub height: f64,
    pub start_frame: f64,
    pub end_frame: f64,
    pub start_timecode: String,
    pub video_tracks: Vec<String>,
    pub audio_tracks: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StoredTimeline {
    pub metadata: TimelineMetadata,
    pub cuts: Vec<Clip>,
    pub audio: Vec<Clip>,
    pub markers: Vec<Marker>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaEntry {
    pub id: String,
    pub name: String,
    pub path: String,
    /// The source's own frame rate, which a clip's source frames count in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
}

/// A default of TRACKED_PROPERTIES.
enum Default {
    Number(f64),
    Bool(bool),
}

// Properties worth versioning and their defaults; Vit tracks the same set.
const TRACKED_PROPERTIES: [(&str, Default); 25] = [
    ("Pan", Default::Number(0.0)),
    ("Tilt", Default::Number(0.0)),
    ("ZoomX", Default::Number(1.0)),
    ("ZoomY", Default::Number(1.0)),
    ("Opacity", Default::Number(100.0)),
    ("RotationAngle", Default::Number(0.0)),
    ("AnchorPointX", Default::Number(0.0)),
    ("AnchorPointY", Default::Number(0.0)),
    ("Pitch", Default::Number(0.0)),
    ("Yaw", Default::Number(0.0)),
    ("FlipX", Default::Bool(false)),
    ("FlipY", Default::Bool(false)),
    ("CropLeft", Default::Number(0.0)),
    ("CropRight", Default::Number(0.0)),
    ("CropTop", Default::Number(0.0)),
    ("CropBottom", Default::Number(0.0)),
    ("CropSoftness", Default::Number(0.0)),
    ("CropRetain", Default::Bool(false)),
    ("Distortion", Default::Number(0.0)),
    ("CompositeMode", Default::Number(0.0)),
    ("DynamicZoomEase", Default::Number(0.0)),
    ("Speed", Default::Number(100.0)),
    ("RetimeProcess", Default::Number(0.0)),
    ("MotionEstimation", Default::Number(0.0)),
    ("Volume", Default::Number(0.0)),
];

fn is_default(value: &Value, fallback: &Default) -> bool {
    match fallback {
        Default::Number(fallback) => js::number(Some(value)) == *fallback,
        Default::Bool(fallback) => js::truthy(value) == *fallback,
    }
}

fn tracked_properties(properties: &Map<String, Value>) -> Option<Map<String, Value>> {
    let mut kept: Vec<(&String, &Value)> = properties
        .iter()
        .filter(|(key, value)| {
            TRACKED_PROPERTIES
                .iter()
                .find(|(name, _)| name == key)
                .is_some_and(|(_, fallback)| !value.is_null() && !is_default(value, fallback))
        })
        .collect();
    if kept.is_empty() {
        return None;
    }
    kept.sort_by(|(a, _), (b, _)| js::locale_compare(a, b));
    Some(
        kept.into_iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect(),
    )
}

/// num(value, fallback): Number(value) when that is finite, else the fallback. None is undefined.
pub fn num(value: Option<&Value>, fallback: f64) -> f64 {
    let number = js::number(value);
    if number.is_finite() { number } else { fallback }
}

fn clips_of(tracks: &[RawTrack]) -> Vec<Clip> {
    tracks
        .iter()
        .flat_map(|track| {
            track.items.iter().map(|item| Clip {
                track: track.index,
                id: item.id.clone(),
                name: item.name.clone().unwrap_or_default(),
                start: num(item.start.as_ref(), 0.0),
                end: num(item.end.as_ref(), 0.0),
                source_start: item.source_start,
                source_end: item.source_end,
                media: item.media.as_ref().map(|media| media.id.clone()),
                enabled: (item.enabled == Some(false)).then_some(false),
                fusion_comps: item.fusion_comps.filter(|n| *n != 0.0 && !n.is_nan()),
                properties: tracked_properties(&item.properties),
            })
        })
        .collect()
}

fn track_names(tracks: &[RawTrack], letter: &str) -> Vec<String> {
    tracks
        .iter()
        .map(|track| {
            track
                .name
                .clone()
                .unwrap_or_else(|| format!("{letter}{}", js::number_to_string(track.index)))
        })
        .collect()
}

pub fn stored_timeline(raw: &RawTimeline) -> StoredTimeline {
    let mut markers: Vec<Marker> = raw
        .markers
        .iter()
        .map(|marker| {
            let text = |key: &str| marker.get(key).map(js::to_string).unwrap_or_default();
            Marker {
                frame: js::number(marker.get("frame")),
                color: text("color"),
                name: text("name"),
                note: text("note"),
                duration: num(marker.get("duration"), 1.0),
            }
        })
        .collect();
    markers.sort_by(|a, b| {
        (a.frame - b.frame)
            .partial_cmp(&0.0)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    StoredTimeline {
        metadata: TimelineMetadata {
            id: raw.id.clone(),
            name: raw.name.clone(),
            frame_rate: num(raw.frame_rate.as_ref(), 24.0),
            width: num(raw.width.as_ref(), 0.0),
            height: num(raw.height.as_ref(), 0.0),
            start_frame: num(raw.start_frame.as_ref(), 0.0),
            end_frame: num(raw.end_frame.as_ref(), 0.0),
            start_timecode: raw.start_timecode.clone().unwrap_or_default(),
            video_tracks: track_names(&raw.video, "V"),
            audio_tracks: track_names(&raw.audio, "A"),
        },
        cuts: clips_of(&raw.video),
        audio: clips_of(&raw.audio),
        markers,
    }
}

pub fn media_entries(snapshot: &RawSnapshot) -> Vec<MediaEntry> {
    // In the order each id was first seen, holding the last entry seen for it, like a Map.
    let mut order: Vec<String> = Vec::new();
    let mut media: HashMap<String, MediaEntry> = HashMap::new();
    for timeline in &snapshot.timelines {
        for track in timeline.video.iter().chain(&timeline.audio) {
            for item in &track.items {
                let Some(raw) = &item.media else { continue };
                let fps = num(raw.fps.as_ref(), 0.0);
                let entry = MediaEntry {
                    id: raw.id.clone(),
                    name: raw.name.clone().unwrap_or_default(),
                    path: raw.path.clone(),
                    fps: (fps != 0.0).then_some(fps),
                };
                if media.insert(raw.id.clone(), entry).is_none() {
                    order.push(raw.id.clone());
                }
            }
        }
    }
    let mut entries: Vec<MediaEntry> = order
        .into_iter()
        .filter_map(|id| media.remove(&id))
        .collect();
    entries.sort_by(|a, b| {
        js::locale_compare(&a.name, &b.name).then_with(|| js::locale_compare(&a.id, &b.id))
    });
    entries
}

/// Rows go one per line, so a git diff or merge of a cut reads as a clip list.
pub fn json_rows<T: Serialize>(key: &str, rows: &[T]) -> String {
    if rows.is_empty() {
        return format!("{{ \"{key}\": [] }}\n");
    }
    let rows: Vec<String> = rows
        .iter()
        .map(|row| format!("  {}", js::stringify(row)))
        .collect();
    format!("{{ \"{key}\": [\n{}\n] }}\n", rows.join(",\n"))
}

pub const TIMELINE_FILES: [&str; 4] = ["metadata.json", "cuts.json", "audio.json", "markers.json"];

/// Each of TIMELINE_FILES with its contents, in that order.
pub fn timeline_files(timeline: &StoredTimeline) -> [(&'static str, String); 4] {
    [
        (
            "metadata.json",
            format!("{}\n", js::stringify_pretty(&timeline.metadata)),
        ),
        ("cuts.json", json_rows("items", &timeline.cuts)),
        ("audio.json", json_rows("items", &timeline.audio)),
        ("markers.json", json_rows("markers", &timeline.markers)),
    ]
}

/// A timeline from its files, read by `read`; None without its metadata.json. A file that isn't JSON of the
/// right shape is an error.
pub fn parse_timeline_files(
    read: impl Fn(&str) -> Option<String>,
) -> anyhow::Result<Option<StoredTimeline>> {
    let Some(metadata) = read("metadata.json") else {
        return Ok(None);
    };
    fn rows<T: serde::de::DeserializeOwned>(
        text: Option<String>,
        key: &str,
    ) -> anyhow::Result<Vec<T>> {
        let Some(text) = text else {
            return Ok(Vec::new());
        };
        let mut parsed: Map<String, Value> = serde_json::from_str(&text)?;
        match parsed.remove(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(rows) => Ok(serde_json::from_value(rows)?),
        }
    }
    Ok(Some(StoredTimeline {
        metadata: serde_json::from_str(&metadata)?,
        cuts: rows(read("cuts.json"), "items")?,
        audio: rows(read("audio.json"), "items")?,
        markers: rows(read("markers.json"), "markers")?,
    }))
}

/// A folder name that is legal on every OS; "Cut: v2?" becomes "Cut_ v2_".
pub fn folder_name(name: &str) -> String {
    let replaced: String = name
        .chars()
        .map(|c| {
            if matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') || c <= '\u{1f}' {
                '_'
            } else {
                c
            }
        })
        .collect();
    let safe = js::trim(replaced.trim_end_matches(['.', ' ']));
    if safe.is_empty() {
        "_".into()
    } else {
        safe.into()
    }
}

/// Folder per timeline, in timeline order; a name that clashes after cleaning gets " (2)", " (3)"…
pub fn timeline_folders<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    let mut used = HashSet::new();
    names
        .iter()
        .map(|name| {
            let base = folder_name(name.as_ref());
            let mut folder = base.clone();
            let mut n = 2;
            while used.contains(&folder.to_lowercase()) {
                folder = format!("{base} ({n})");
                n += 1;
            }
            used.insert(folder.to_lowercase());
            folder
        })
        .collect()
}
