//! Cut-level edits to a .drp or .drt. Each edit splices the few XML elements it changes; the rest of every
//! entry, blobs included, stays exactly as Resolve wrote it, and entries no edit touches are copied byte for byte.

use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use indexmap::IndexMap;
use regex::Regex;
use serde::{Deserialize, Serialize};

use crate::blobs::{decode_in, encode_in, with_clip_path};
use crate::read::{
    DrpFile, MP_FOLDER, SEQ_CONTAINER, TrackType, clip_elements, frames_of, js_round, media_items,
    read_drp, track_elements,
};
use crate::xml::{
    Splice, XmlElement, apply_splices, child, child_text, descendants, parse_xml, set_text, text_of,
};
use crate::zip::write_zip;
use crate::{Error, Result, fail};

/// One edit, shaped like the TS `Edit` union on the wire: `{ "type": "moveClip", "clipId": …, … }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Edit {
    /// Puts the clip at record frame `start`, on track `track_id` of the same timeline and type if given.
    MoveClip {
        clip_id: String,
        start: i64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        track_id: Option<String>,
    },
    /// Sets the clip's record in and/or out (out exclusive). Moving the in point moves the source in with it.
    TrimClip {
        clip_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        start: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        end: Option<i64>,
    },
    RemoveClip {
        clip_id: String,
    },
    /// Renames a timeline or compound clip, by its media pool item's id.
    RenameTimeline {
        timeline_id: String,
        name: String,
    },
    /// Points a media pool item at another file, by rewriting the path in its `<Clip>` blobs.
    RelinkMedia {
        media_id: String,
        path: String,
    },
}

/// The file with `edits` applied in order, as a new .drp/.drt. Errs, and changes nothing, when an edit names
/// something that isn't there or would leave clips overlapping on a track.
pub fn patch_drp(file: &DrpFile, edits: &[Edit]) -> Result<Vec<u8>> {
    let mut state = Texts::new(&file.xml);
    let mut tracks = HashSet::new();
    for edit in edits {
        match edit {
            Edit::MoveClip {
                clip_id,
                start,
                track_id,
            } => {
                tracks.insert(move_clip(&mut state, clip_id, *start, track_id.as_deref())?);
            }
            Edit::TrimClip {
                clip_id,
                start,
                end,
            } => {
                tracks.insert(trim_clip(&mut state, file, clip_id, *start, *end)?);
            }
            Edit::RemoveClip { clip_id } => remove_clip(&mut state, clip_id)?,
            Edit::RenameTimeline { timeline_id, name } => {
                rename_timeline(&mut state, timeline_id, name)?
            }
            Edit::RelinkMedia { media_id, path } => relink_media(&mut state, media_id, path)?,
        }
    }

    let mut changes = IndexMap::new();
    for (entry, text) in &state.texts {
        if file.xml.get(entry) != Some(text) {
            changes.insert(entry.clone(), text.clone().into_bytes());
        }
    }
    let buffer = write_zip(&file.zip, &changes)?;
    check_tracks(&read_drp(&buffer)?, &tracks)?;
    Ok(buffer)
}

/// The XML entries as edited so far, each parsed again only after it changes.
struct Texts {
    texts: IndexMap<String, String>,
    roots: HashMap<String, Rc<XmlElement>>,
}

impl Texts {
    fn new(xml: &IndexMap<String, String>) -> Self {
        Self {
            texts: xml.clone(),
            roots: HashMap::new(),
        }
    }

    fn root(&mut self, entry: &str) -> Result<Rc<XmlElement>> {
        if let Some(root) = self.roots.get(entry) {
            return Ok(root.clone());
        }
        let root = Rc::new(parse_xml(self.text(entry))?);
        self.roots.insert(entry.to_string(), root.clone());
        Ok(root)
    }

    fn text(&self, entry: &str) -> &str {
        &self.texts[entry]
    }

    fn splice(&mut self, entry: &str, splices: &[Splice]) -> Result<()> {
        let text = apply_splices(self.text(entry), splices)?;
        self.texts.insert(entry.to_string(), text);
        self.roots.remove(entry);
        Ok(())
    }

    fn entries(&self, pattern: &Regex) -> Vec<String> {
        self.texts
            .keys()
            .filter(|entry| pattern.is_match(entry))
            .cloned()
            .collect()
    }
}

/// Where a clip sits. Elements are copies from one parse, so their offsets identify them.
struct ClipAt {
    entry: String,
    track: XmlElement,
    items: XmlElement,
    /// The `<Element>` in `<Items>` that holds the clip.
    holder: XmlElement,
    clip: XmlElement,
}

fn find_clip(state: &mut Texts, clip_id: &str) -> Result<ClipAt> {
    for entry in state.entries(&SEQ_CONTAINER) {
        let root = state.root(&entry)?;
        for track in track_elements(&root) {
            let Some(items) = child(track, "Items") else {
                continue;
            };
            for holder in &items.children {
                if let Some(clip) = holder
                    .children
                    .iter()
                    .find(|element| element.id.as_deref() == Some(clip_id))
                {
                    return Ok(ClipAt {
                        entry,
                        track: track.clone(),
                        items: items.clone(),
                        holder: holder.clone(),
                        clip: clip.clone(),
                    });
                }
            }
        }
    }
    fail(format!("There's no clip {clip_id} on any timeline."))
}

fn frames(value: i64, what: &str) -> Result<i64> {
    const MAX_SAFE_INTEGER: i64 = (1 << 53) - 1;
    if !(0..=MAX_SAFE_INTEGER).contains(&value) {
        return fail(format!(
            "The {what} must be a whole number of frames, not {value}."
        ));
    }
    Ok(value)
}

/// A clip's child element that every clip Resolve writes has.
fn required<'a>(clip: &'a XmlElement, name: &str) -> Result<&'a XmlElement> {
    child(clip, name).ok_or_else(|| {
        Error(format!(
            "Clip {} has no <{name}>.",
            clip.id.as_deref().unwrap_or("")
        ))
    })
}

fn move_clip(
    state: &mut Texts,
    clip_id: &str,
    start: i64,
    track_id: Option<&str>,
) -> Result<String> {
    frames(start, "start")?;
    let at = find_clip(state, clip_id)?;
    let text = state.text(&at.entry).to_string();
    let mut target = at.track.clone();
    if let Some(track_id) = track_id
        && Some(track_id) != at.track.id.as_deref()
    {
        let root = state.root(&at.entry)?;
        let Some(found) = track_elements(&root)
            .into_iter()
            .find(|track| track.id.as_deref() == Some(track_id))
        else {
            return fail(format!("Track {track_id} isn't on the clip's timeline."));
        };
        if child_text(&text, found, "Type") != child_text(&text, &at.track, "Type") {
            return fail(
                "A clip can only move to a track of its own type: video to video, audio to audio.",
            );
        }
        target = found.clone();
    }
    let start_splice = set_text(&text, required(&at.clip, "Start")?, &start.to_string())?;

    // Same track and the clips stay in order: only <Start> changes.
    let clips = clip_elements(&at.track);
    let others: Vec<_> = clips
        .iter()
        .filter(|clip| clip.start != at.clip.start)
        .collect();
    let index = others
        .iter()
        .position(|clip| frames_of(&child_text(&text, clip, "Start")) > start);
    let before = clips.iter().position(|clip| clip.start == at.clip.start);
    if target.start == at.track.start && Some(index.unwrap_or(others.len())) == before {
        state.splice(&at.entry, &[start_splice])?;
        return Ok(target.id.unwrap_or_default());
    }

    // Otherwise the clip's <Element> moves, keeping each track's clips in record order like Resolve writes them.
    let (from, to) = line_block(&text, &at.holder);
    let shifted = Splice {
        start: start_splice.start - from,
        end: start_splice.end - from,
        text: start_splice.text,
    };
    let block = apply_splices(&text[from..to], &[shifted])?;
    state.splice(&at.entry, &[removal(&at.items, from, to)])?;

    let after = state.text(&at.entry).to_string();
    let root = state.root(&at.entry)?;
    let target_name = target.id.as_deref().unwrap_or("null");
    let Some(track) = track_elements(&root)
        .into_iter()
        .find(|candidate| candidate.id == target.id)
    else {
        return fail(format!("Track {target_name} isn't on the clip's timeline."));
    };
    let Some(items) = child(track, "Items") else {
        return fail(format!("Track {target_name} has no <Items>."));
    };
    let next = items.children.iter().find(|holder| {
        holder
            .children
            .first()
            .is_some_and(|clip| frames_of(&child_text(&after, clip, "Start")) > start)
    });
    let insert = if let Some(next) = next {
        let line = line_start(&after, next.start);
        Splice {
            start: line,
            end: line,
            text: block,
        }
    } else if items.inner_start == items.end {
        let indent = &after[line_start(&after, items.start)..items.start];
        let name = &items.name;
        Splice {
            start: items.start,
            end: items.end,
            text: format!("<{name}>{}{block}{indent}</{name}>", newline(&after)),
        }
    } else {
        let line = line_start(&after, items.inner_end);
        Splice {
            start: line,
            end: line,
            text: block,
        }
    };
    state.splice(&at.entry, &[insert])?;
    Ok(target.id.unwrap_or_default())
}

fn trim_clip(
    state: &mut Texts,
    file: &DrpFile,
    clip_id: &str,
    start: Option<i64>,
    end: Option<i64>,
) -> Result<String> {
    let at = find_clip(state, clip_id)?;
    let text = state.text(&at.entry).to_string();
    let old_start = frames_of(&child_text(&text, &at.clip, "Start"));
    let old_end = old_start + frames_of(&child_text(&text, &at.clip, "Duration"));
    let new_start = frames(start.unwrap_or(old_start), "start")?;
    let new_end = frames(end.unwrap_or(old_end), "end")?;
    if new_end <= new_start {
        return fail(format!(
            "A clip needs at least one frame: {new_start} to {new_end} has none."
        ));
    }

    let mut splices = vec![
        set_text(&text, required(&at.clip, "Start")?, &new_start.to_string())?,
        set_text(
            &text,
            required(&at.clip, "Duration")?,
            &(new_end - new_start).to_string(),
        )?,
    ];
    let mut source_in = 0;
    if let Some(in_element) = child(&at.clip, "In") {
        source_in = decode_in(&text_of(&text, Some(in_element))).map_err(|_| {
            Error(format!("Clip {clip_id} has a source in point this library can't read, so it can't be trimmed."))
        })?;
        source_in += new_start - old_start;
        if source_in < 0 {
            return fail(format!(
                "That trim starts {} frame(s) before the clip's media does.",
                -source_in
            ));
        }
        if new_start != old_start {
            splices.push(set_text(&text, in_element, &encode_in(source_in))?);
        }
    } else if new_start != old_start {
        return fail(format!(
            "Clip {clip_id} has no <In>, so its in point can't move."
        ));
    }

    // Past the end of its media the clip would play nothing; refuse when the media's length is known.
    let timeline = file
        .timelines
        .iter()
        .find(|candidate| candidate.entry == at.entry);
    let media_id = child_text(&text, &at.clip, "MediaRef");
    let media = media_items(&file.media_pool)
        .into_iter()
        .find(|item| item.id == media_id);
    let known = |value: f64| value != 0.0 && !value.is_nan();
    if let (Some(timeline_fps), Some(media)) = (timeline.and_then(|timeline| timeline.fps), media)
        && let (Some(media_fps), Some(media_frames)) = (media.fps, media.frames)
        && known(timeline_fps)
        && known(media_fps)
        && media_frames != 0
    {
        let length = js_round(media_frames as f64 * timeline_fps / media_fps);
        if source_in + new_end - new_start > length {
            return fail(format!(
                "That trim runs past the end of \"{}\", which is {length} frame(s) long on this timeline.",
                media.name
            ));
        }
    }
    state.splice(&at.entry, &splices)?;
    Ok(at.track.id.unwrap_or_default())
}

fn remove_clip(state: &mut Texts, clip_id: &str) -> Result<()> {
    let at = find_clip(state, clip_id)?;
    let text = state.text(&at.entry).to_string();
    let inside = text[at.clip.start..at.clip.end].matches(clip_id).count();
    let everywhere: usize = state
        .texts
        .values()
        .map(|entry| entry.matches(clip_id).count())
        .sum();
    if everywhere > inside {
        return fail(format!(
            "Something else in the file refers to clip {clip_id}, so removing it would break the file."
        ));
    }
    let (from, to) = line_block(&text, &at.holder);
    state.splice(&at.entry, &[removal(&at.items, from, to)])
}

fn rename_timeline(state: &mut Texts, timeline_id: &str, name: &str) -> Result<()> {
    if name.trim().is_empty() {
        return fail("A timeline needs a name.");
    }
    let (entry, item) = find_media_item(state, timeline_id)?;
    let text = state.text(&entry).to_string();
    if !item
        .children
        .iter()
        .any(|element| element.name == "Sequence" || element.name == "TimelineSharedHandle")
    {
        return fail(format!(
            "Media pool item {timeline_id} isn't a timeline or compound clip."
        ));
    }
    for other in state.entries(&MP_FOLDER) {
        let root = state.root(&other)?;
        let other_text = state.text(&other);
        for holder in child(&root, "MediaVec")
            .map(|vec| vec.children.as_slice())
            .unwrap_or_default()
        {
            for element in &holder.children {
                if element.id.as_deref() != Some(timeline_id)
                    && element.name == item.name
                    && child_text(other_text, element, "Name") == name
                {
                    let what = if item.name == "Sm2MpCompoundClip" {
                        "compound clip"
                    } else {
                        "timeline"
                    };
                    return fail(format!("There's already a {what} called \"{name}\"."));
                }
            }
        }
    }
    // The media pool item and the Sm2Timeline inside it each carry the name.
    let names = std::iter::once(child(&item, "Name")).chain(
        descendants(&item)
            .into_iter()
            .filter(|element| element.name == "Sm2Timeline")
            .map(|timeline| child(timeline, "Name")),
    );
    let splices = names
        .flatten()
        .map(|element| set_text(&text, element, name))
        .collect::<Result<Vec<_>>>()?;
    state.splice(&entry, &splices)
}

fn relink_media(state: &mut Texts, media_id: &str, path: &str) -> Result<()> {
    let (entry, item) = find_media_item(state, media_id)?;
    let text = state.text(&entry).to_string();
    let blobs: Vec<&XmlElement> = descendants(&item)
        .into_iter()
        .filter(|element| element.name == "BtVideoInfo" || element.name == "BtAudioInfo")
        .filter_map(|info| child(info, "Clip"))
        .filter(|clip| !text_of(&text, Some(clip)).is_empty())
        .collect();
    if blobs.is_empty() {
        return fail(format!("Media pool item {media_id} has no file to relink."));
    }
    let splices = blobs
        .iter()
        .map(|clip| {
            set_text(
                &text,
                clip,
                &with_clip_path(&text_of(&text, Some(clip)), path)?,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    state.splice(&entry, &splices)?;

    // The clips' plain-text paths too, so the file agrees with itself.
    for seq in state.entries(&SEQ_CONTAINER) {
        let root = state.root(&seq)?;
        let seq_text = state.text(&seq);
        let mut splices = Vec::new();
        for track in track_elements(&root) {
            for clip in clip_elements(track) {
                if let Some(file_path) = child(clip, "MediaFilePath")
                    && child_text(seq_text, clip, "MediaRef") == media_id
                {
                    splices.push(set_text(seq_text, file_path, path)?);
                }
            }
        }
        if !splices.is_empty() {
            state.splice(&seq, &splices)?;
        }
    }
    Ok(())
}

fn find_media_item(state: &mut Texts, id: &str) -> Result<(String, XmlElement)> {
    for entry in state.entries(&MP_FOLDER) {
        let root = state.root(&entry)?;
        for holder in child(&root, "MediaVec")
            .map(|vec| vec.children.as_slice())
            .unwrap_or_default()
        {
            if let Some(item) = holder
                .children
                .iter()
                .find(|element| element.id.as_deref() == Some(id))
            {
                return Ok((entry, item.clone()));
            }
        }
    }
    fail(format!("There's no media pool item {id}."))
}

/// Refuses a result where clips overlap, or have no frames, on a track an edit changed.
fn check_tracks(file: &DrpFile, track_ids: &HashSet<String>) -> Result<()> {
    for timeline in &file.timelines {
        for track in &timeline.tracks {
            if !track_ids.contains(&track.id) {
                continue;
            }
            let mut clips: Vec<_> = track.clips.iter().collect();
            clips.sort_by_key(|clip| clip.start);
            for pair in clips.windows(2) {
                if pair[1].start < pair[0].end {
                    let label = format!(
                        "{}{}",
                        if track.kind == TrackType::Audio {
                            "A"
                        } else {
                            "V"
                        },
                        track.index
                    );
                    return fail(format!(
                        "That edit makes \"{}\" overlap \"{}\" on {label} of \"{}\".",
                        pair[1].name, pair[0].name, timeline.name
                    ));
                }
            }
        }
    }
    Ok(())
}

/// Taking text[from, to) out of `items`, or the whole list down to `<Items/>` when it was the last clip.
fn removal(items: &XmlElement, from: usize, to: usize) -> Splice {
    if items.children.len() == 1 {
        return Splice {
            start: items.start,
            end: items.end,
            text: format!("<{}/>", items.name),
        };
    }
    Splice {
        start: from,
        end: to,
        text: String::new(),
    }
}

/// An element's whole lines, indentation and line break included, when it has them to itself.
fn line_block(text: &str, element: &XmlElement) -> (usize, usize) {
    let bytes = text.as_bytes();
    let from = line_start(text, element.start);
    if from == element.start && from > 0 && bytes[from - 1] != b'\n' {
        return (element.start, element.end);
    }
    // Spaces or tabs, then a line break, within the next 64 characters.
    let window = &bytes[element.end..(element.end + 64).min(bytes.len())];
    let blanks = window
        .iter()
        .take_while(|&&byte| byte == b' ' || byte == b'\t')
        .count();
    let rest = match &window[blanks..] {
        [b'\r', b'\n', ..] => blanks + 2,
        [b'\n', ..] => blanks + 1,
        _ => 0,
    };
    (from, element.end + rest)
}

/// Where the line holding `at` starts, if only indentation comes before `at` on it; otherwise `at`.
fn line_start(text: &str, at: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = at;
    while i > 0 && (bytes[i - 1] == b' ' || bytes[i - 1] == b'\t') {
        i -= 1;
    }
    if i == 0 || bytes[i - 1] == b'\n' {
        i
    } else {
        at
    }
}

fn newline(text: &str) -> &'static str {
    if text.contains("\r\n") { "\r\n" } else { "\n" }
}
