// What changed between two saves: one line per timeline for the commit message, and the strips of clips the
// history panel shows.

use std::collections::{HashMap, HashSet};

use linker_edit_protocol::{ClipChange, StripTile, TimelineStrip, TimelineStripState};
use serde::Serialize;

use super::model::{Clip, Marker, StoredTimeline};
use crate::js;

pub fn format_duration(frames: f64, fps: f64) -> String {
    let fps = if fps == 0.0 || fps.is_nan() {
        24.0
    } else {
        fps
    };
    let seconds = js::math_round(frames / fps).max(0.0) as u64;
    let h = seconds / 3600;
    let m = (seconds % 3600) / 60;
    let s = seconds % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn length(timeline: &StoredTimeline) -> String {
    format_duration(
        timeline.metadata.end_frame - timeline.metadata.start_frame,
        timeline.metadata.frame_rate,
    )
}

fn count(n: usize, singular: &str, plural: Option<&str>) -> String {
    if n == 1 {
        format!("{n} {singular}")
    } else {
        match plural {
            Some(plural) => format!("{n} {plural}"),
            None => format!("{n} {singular}s"),
        }
    }
}

/// "3 clips removed (A001, A002, B003)": names only while the list stays short.
fn phrase(clips: &[&Clip], noun: &str, verb: &str) -> Option<String> {
    if clips.is_empty() {
        return None;
    }
    let names = if clips.len() <= 3 {
        let names: Vec<&str> = clips
            .iter()
            .map(|clip| {
                if clip.name.is_empty() {
                    "untitled"
                } else {
                    clip.name.as_str()
                }
            })
            .collect();
        format!(" ({})", names.join(", "))
    } else {
        String::new()
    };
    Some(format!("{} {verb}{names}", count(clips.len(), noun, None)))
}

/// JSON.stringify(a ?? null) === JSON.stringify(b ?? null).
fn same_json<T: Serialize>(a: &T, b: &T) -> bool {
    js::stringify(a) == js::stringify(b)
}

struct ComparedClip<'a> {
    clip: &'a Clip,
    previous: Option<&'a Clip>,
    change: Option<ClipChange>,
}

/// A Map by id as JavaScript has one: each id in the order it was first set, holding the last value set for it.
struct OrderedById<'a> {
    order: Vec<&'a str>,
    values: HashMap<&'a str, &'a Clip>,
}

impl<'a> OrderedById<'a> {
    fn new(clips: &'a [Clip]) -> Self {
        let mut map = OrderedById {
            order: Vec::new(),
            values: HashMap::new(),
        };
        for clip in clips {
            if map.values.insert(&clip.id, clip).is_none() {
                map.order.push(&clip.id);
            }
        }
        map
    }

    fn delete(&mut self, id: &str) {
        if self.values.remove(id).is_some() {
            self.order.retain(|other| *other != id);
        }
    }

    fn values(&self) -> impl Iterator<Item = &'a Clip> + '_ {
        self.order.iter().map(|id| self.values[id])
    }
}

/// Every clip after the change, then the removed ones, each with what happened to it. Trimmed beats moved beats changed.
fn compare_clips<'a>(before: &'a [Clip], after: &'a [Clip]) -> Vec<ComparedClip<'a>> {
    let mut old = OrderedById::new(before);
    let unmatched: Vec<usize> = (0..after.len())
        .filter(|&index| !old.values.contains_key(after[index].id.as_str()))
        .collect();
    let matched: HashMap<&str, &Clip> = after
        .iter()
        .filter_map(|clip| {
            old.values
                .get(clip.id.as_str())
                .map(|was| (clip.id.as_str(), *was))
        })
        .collect();
    for was in matched.values() {
        old.delete(&was.id);
    }
    // Resolve gives a clip a new id whenever it is placed again, which is how scripts and agents move or
    // re-trim one. The same media from the same source in or out point is still that clip.
    let mut pairs: HashMap<usize, &Clip> = HashMap::new();
    for index in unmatched {
        let clip = &after[index];
        let twin = old.values().find(|candidate| {
            clip.media.is_some()
                && candidate.media == clip.media
                && (candidate.source_start == clip.source_start
                    || candidate.source_end == clip.source_end)
        });
        let Some(twin) = twin else { continue };
        old.delete(&twin.id);
        pairs.insert(index, twin);
    }
    let mut compared: Vec<ComparedClip> = after
        .iter()
        .enumerate()
        .map(|(index, clip)| {
            let previous = matched
                .get(clip.id.as_str())
                .copied()
                .or_else(|| pairs.get(&index).copied());
            let Some(previous) = previous else {
                return ComparedClip {
                    clip,
                    previous: None,
                    change: Some(ClipChange::Added),
                };
            };
            let change = if previous.end - previous.start != clip.end - clip.start
                || previous.source_start != clip.source_start
                || previous.source_end != clip.source_end
            {
                Some(ClipChange::Trimmed)
            } else if previous.start != clip.start || previous.track != clip.track {
                Some(ClipChange::Moved)
            } else if previous.media != clip.media
                || previous.name != clip.name
                || previous.enabled != clip.enabled
                || previous.fusion_comps != clip.fusion_comps
                || !same_json(&previous.properties, &clip.properties)
            {
                Some(ClipChange::Changed)
            } else {
                None
            };
            ComparedClip {
                clip,
                previous: Some(previous),
                change,
            }
        })
        .collect();
    let remaining: HashSet<&str> = old.order.iter().copied().collect();
    compared.extend(
        before
            .iter()
            .filter(|clip| remaining.contains(clip.id.as_str()))
            .map(|clip| ComparedClip {
                clip,
                previous: None,
                change: Some(ClipChange::Removed),
            }),
    );
    compared
}

const VERBS: [(ClipChange, &str); 5] = [
    (ClipChange::Added, "added"),
    (ClipChange::Removed, "removed"),
    (ClipChange::Trimmed, "trimmed"),
    (ClipChange::Moved, "moved"),
    (ClipChange::Changed, "changed"),
];

fn clip_changes(before: &[Clip], after: &[Clip], noun: &str) -> Vec<String> {
    let compared = compare_clips(before, after);
    VERBS
        .iter()
        .filter_map(|(change, verb)| {
            let clips: Vec<&Clip> = compared
                .iter()
                .filter(|entry| entry.change == Some(*change))
                .map(|entry| entry.clip)
                .collect();
            phrase(&clips, noun, verb)
        })
        .collect()
}

fn marker_changes(before: &[Marker], after: &[Marker]) -> Vec<String> {
    // Keyed by frame as JavaScript writes the number, so 10 and 10.0 are one frame.
    let key = |marker: &Marker| js::number_to_string(marker.frame);
    let mut old: HashMap<String, &Marker> = HashMap::new();
    for marker in before {
        old.insert(key(marker), marker);
    }
    let mut added = 0;
    let mut changed = 0;
    for marker in after {
        match old.remove(&key(marker)) {
            None => added += 1,
            Some(previous) if !same_json(previous, marker) => changed += 1,
            Some(_) => {}
        }
    }
    let mut parts = Vec::new();
    if added > 0 {
        parts.push(format!("{} added", count(added, "marker", None)));
    }
    if !old.is_empty() {
        parts.push(format!("{} removed", count(old.len(), "marker", None)));
    }
    if changed > 0 {
        parts.push(format!("{} changed", count(changed, "marker", None)));
    }
    parts
}

fn timeline_changes(before: &StoredTimeline, after: &StoredTimeline) -> Vec<String> {
    let a = &before.metadata;
    let b = &after.metadata;
    let mut parts = Vec::new();
    if a.name != b.name {
        parts.push(format!("renamed from \"{}\"", a.name));
    }
    parts.extend(clip_changes(&before.cuts, &after.cuts, "clip"));
    parts.extend(clip_changes(&before.audio, &after.audio, "audio clip"));
    parts.extend(marker_changes(&before.markers, &after.markers));
    if !same_json(&a.video_tracks, &b.video_tracks) || !same_json(&a.audio_tracks, &b.audio_tracks)
    {
        parts.push("tracks changed".into());
    }
    if a.frame_rate != b.frame_rate || a.width != b.width || a.height != b.height {
        parts.push("timeline settings changed".into());
    }
    if !parts.is_empty() && length(before) != length(after) {
        parts.push(format!("length {} → {}", length(before), length(after)));
    }
    parts
}

/// One line per timeline that changed, matched by Resolve's timeline id so renames read as renames.
pub fn describe_changes(before: &[StoredTimeline], after: &[StoredTimeline]) -> Vec<String> {
    let mut old_order: Vec<&str> = Vec::new();
    let mut old: HashMap<&str, &StoredTimeline> = HashMap::new();
    for timeline in before {
        if old.insert(&timeline.metadata.id, timeline).is_none() {
            old_order.push(&timeline.metadata.id);
        }
    }
    let mut lines = Vec::new();
    for timeline in after {
        let previous = old.remove(timeline.metadata.id.as_str());
        let name = format!("\"{}\"", timeline.metadata.name);
        let Some(previous) = previous else {
            lines.push(format!(
                "{name}: new timeline, {}, {}",
                count(timeline.cuts.len(), "clip", None),
                length(timeline)
            ));
            continue;
        };
        let parts = timeline_changes(previous, timeline);
        if !parts.is_empty() {
            lines.push(format!("{name}: {}", parts.join(", ")));
        }
    }
    for id in old_order {
        if let Some(timeline) = old.get(id) {
            lines.push(format!("\"{}\": timeline removed", timeline.metadata.name));
        }
    }
    lines
}

/// Tiles shown before a strip folds the rest into "+N".
const STRIP_LIMIT: usize = 24;

fn seconds(frames: f64, fps: f64) -> f64 {
    let fps = if fps == 0.0 || fps.is_nan() {
        24.0
    } else {
        fps
    };
    js::math_round((frames / fps) * 10.0) / 10.0
}

/// Video clips in timeline order for the history panel. A new timeline shows its whole cut; a changed
/// one shows each changed clip with a neighbour either side and folds the untouched runs between.
pub fn timeline_strips(
    before: &[StoredTimeline],
    after: &[StoredTimeline],
    is_image: impl Fn(&str) -> bool,
) -> Vec<TimelineStrip> {
    let mut old: HashMap<&str, &StoredTimeline> = HashMap::new();
    for timeline in before {
        old.insert(&timeline.metadata.id, timeline);
    }
    let mut strips = Vec::new();
    for timeline in after {
        let previous = old.get(timeline.metadata.id.as_str()).copied();
        let fps = timeline.metadata.frame_rate;
        let mut compared = match previous {
            Some(previous) => compare_clips(&previous.cuts, &timeline.cuts),
            None => timeline
                .cuts
                .iter()
                .map(|clip| ComparedClip {
                    clip,
                    previous: None,
                    change: None,
                })
                .collect(),
        };
        if previous.is_some() && !compared.iter().any(|entry| entry.change.is_some()) {
            continue;
        }
        if compared.is_empty() {
            continue;
        }

        compared.sort_by(|a, b| {
            let order = (a.clip.start - b.clip.start).partial_cmp(&0.0);
            let order = match order {
                Some(std::cmp::Ordering::Equal) | None => {
                    (a.clip.track - b.clip.track).partial_cmp(&0.0)
                }
                order => order,
            };
            order.unwrap_or(std::cmp::Ordering::Equal)
        });
        let tile = |entry: &ComparedClip| -> StripTile {
            let delta = match (entry.change, entry.previous) {
                (Some(ClipChange::Moved), Some(was)) => {
                    Some(seconds(entry.clip.start - was.start, fps))
                }
                (Some(ClipChange::Trimmed), Some(was)) => Some(seconds(
                    entry.clip.end - entry.clip.start - (was.end - was.start),
                    fps,
                )),
                _ => None,
            };
            StripTile::Clip {
                name: entry.clip.name.clone(),
                media: entry.clip.media.clone(),
                image: entry.clip.media.as_deref().is_some_and(&is_image),
                change: entry.change,
                delta: delta.filter(|delta| *delta != 0.0 && !delta.is_nan()),
            }
        };

        let mut keep: Vec<bool> = (0..compared.len())
            .map(|index| {
                previous.is_none()
                    || [index.checked_sub(1), Some(index), Some(index + 1)]
                        .into_iter()
                        .flatten()
                        .any(|near| {
                            compared
                                .get(near)
                                .is_some_and(|entry| entry.change.is_some())
                        })
            })
            .collect();
        let mut tiles = Vec::new();
        let mut shown = 0;
        let mut index = 0;
        while index < compared.len() {
            let mut run = 0;
            while index + run < compared.len() && !keep[index + run] {
                run += 1;
            }
            // A lone untouched clip shows as itself: a "+1" tile would take the same room.
            if run == 1 {
                keep[index] = true;
            }
            if run > 1 {
                tiles.push(StripTile::More { count: run as u32 });
                index += run;
                continue;
            }
            if shown == STRIP_LIMIT {
                tiles.push(StripTile::More {
                    count: (compared.len() - index) as u32,
                });
                break;
            }
            tiles.push(tile(&compared[index]));
            shown += 1;
            index += 1;
        }
        strips.push(TimelineStrip {
            id: timeline.metadata.id.clone(),
            name: timeline.metadata.name.clone(),
            state: if previous.is_some() {
                TimelineStripState::Changed
            } else {
                TimelineStripState::New
            },
            tiles,
        });
    }
    strips
}

#[cfg(test)]
mod tests {
    use super::super::model::{Marker, TimelineMetadata, timeline_folders};
    use super::*;

    /// A named case: the timelines before and after, and what comes out.
    type Case<T> = (
        &'static str,
        Vec<StoredTimeline>,
        Vec<StoredTimeline>,
        Vec<T>,
    );

    fn clip(id: &str) -> Clip {
        Clip {
            track: 1.0,
            id: id.into(),
            name: id.into(),
            start: 0.0,
            end: 100.0,
            source_start: Some(0.0),
            source_end: Some(100.0),
            media: Some(format!("m-{id}")),
            enabled: None,
            fusion_comps: None,
            properties: None,
        }
    }

    fn at(id: &str, start: f64, end: f64) -> Clip {
        Clip {
            start,
            end,
            ..clip(id)
        }
    }

    fn timeline_named(name: &str, end_frame: f64) -> StoredTimeline {
        StoredTimeline {
            metadata: TimelineMetadata {
                id: "t1".into(),
                name: name.into(),
                frame_rate: 24.0,
                width: 1920.0,
                height: 1080.0,
                start_frame: 0.0,
                end_frame,
                start_timecode: "01:00:00:00".into(),
                video_tracks: vec!["V1".into()],
                audio_tracks: vec!["A1".into()],
            },
            cuts: vec![clip("a"), at("b", 100.0, 200.0)],
            audio: vec![],
            markers: vec![],
        }
    }

    fn timeline() -> StoredTimeline {
        timeline_named("Cut", 2400.0)
    }

    fn with_cuts(cuts: Vec<Clip>) -> StoredTimeline {
        StoredTimeline { cuts, ..timeline() }
    }

    fn marker(frame: f64, color: &str, name: &str) -> Marker {
        Marker {
            frame,
            color: color.into(),
            name: name.into(),
            note: String::new(),
            duration: 1.0,
        }
    }

    fn properties(value: serde_json::Value) -> Option<serde_json::Map<String, serde_json::Value>> {
        value.as_object().cloned()
    }

    #[test]
    fn describe_changes_cases() {
        let cases: Vec<Case<&str>> = vec![
            (
                "nothing changed",
                vec![timeline()],
                vec![timeline()],
                vec![],
            ),
            (
                "a new timeline",
                vec![],
                vec![timeline()],
                vec!["\"Cut\": new timeline, 2 clips, 1:40"],
            ),
            (
                "a removed timeline",
                vec![timeline()],
                vec![],
                vec!["\"Cut\": timeline removed"],
            ),
            (
                "a rename, matched by id",
                vec![timeline()],
                vec![timeline_named("Cut v2", 2400.0)],
                vec!["\"Cut v2\": renamed from \"Cut\""],
            ),
            (
                "clips added and removed, with the new length",
                vec![timeline()],
                vec![StoredTimeline {
                    cuts: vec![clip("a"), clip("c"), clip("d")],
                    ..timeline_named("Cut", 1200.0)
                }],
                vec!["\"Cut\": 2 clips added (c, d), 1 clip removed (b), length 1:40 → 0:50"],
            ),
            (
                "trimmed beats moved, moved beats changed",
                vec![timeline()],
                vec![with_cuts(vec![
                    Clip {
                        source_start: Some(10.0),
                        ..clip("a")
                    },
                    Clip {
                        properties: properties(serde_json::json!({ "ZoomX": 2 })),
                        ..at("b", 300.0, 400.0)
                    },
                ])],
                vec!["\"Cut\": 1 clip trimmed (a), 1 clip moved (b)"],
            ),
            (
                "property and enabled changes",
                vec![timeline()],
                vec![with_cuts(vec![
                    Clip {
                        properties: properties(serde_json::json!({ "Opacity": 50 })),
                        ..clip("a")
                    },
                    Clip {
                        enabled: Some(false),
                        ..at("b", 100.0, 200.0)
                    },
                ])],
                vec!["\"Cut\": 2 clips changed (a, b)"],
            ),
            (
                "a clip placed again under a new id is still that clip, matched by media and source point",
                vec![timeline()],
                vec![with_cuts(vec![
                    clip("a"),
                    Clip {
                        media: Some("m-b".into()),
                        ..at("b2", 148.0, 248.0)
                    },
                    Clip {
                        media: Some("m-b".into()),
                        source_start: Some(500.0),
                        source_end: Some(600.0),
                        ..clip("c")
                    },
                ])],
                vec!["\"Cut\": 1 clip added (c), 1 clip moved (b2)"],
            ),
            (
                "audio clips are counted apart from video",
                vec![StoredTimeline {
                    audio: vec![clip("x")],
                    ..timeline()
                }],
                vec![timeline()],
                vec!["\"Cut\": 1 audio clip removed (x)"],
            ),
            (
                "long lists drop the names",
                vec![with_cuts(vec![])],
                vec![with_cuts(["p", "q", "r", "s"].map(clip).to_vec())],
                vec!["\"Cut\": 4 clips added"],
            ),
            (
                "markers by frame",
                vec![StoredTimeline {
                    markers: vec![marker(10.0, "Blue", "a")],
                    ..timeline()
                }],
                vec![StoredTimeline {
                    markers: vec![marker(10.0, "Red", "a"), marker(20.0, "Blue", "b")],
                    ..timeline()
                }],
                vec!["\"Cut\": 1 marker added, 1 marker changed"],
            ),
        ];
        for (name, before, after, expected) in cases {
            assert_eq!(describe_changes(&before, &after), expected, "{name}");
        }
    }

    /// Ten 1-second clips back to back at 24 fps; "c3" is a still.
    fn row(count: usize) -> Vec<Clip> {
        (0..count)
            .map(|i| at(&format!("c{i}"), (i * 24) as f64, (i * 24 + 24) as f64))
            .collect()
    }

    fn is_image(media: &str) -> bool {
        media == "m-c3"
    }

    /// "c3:moved2" for a clip, "+4" for folded clips.
    fn short(
        before: &[StoredTimeline],
        after: &[StoredTimeline],
    ) -> Vec<(TimelineStripState, String)> {
        timeline_strips(before, after, is_image)
            .into_iter()
            .map(|strip| {
                let tiles: Vec<String> = strip
                    .tiles
                    .iter()
                    .map(|tile| match tile {
                        StripTile::More { count } => format!("+{count}"),
                        StripTile::Clip {
                            name,
                            change,
                            delta,
                            ..
                        } => {
                            let change = change
                                .map(|change| {
                                    format!(
                                        ":{}",
                                        serde_json::to_value(change).unwrap().as_str().unwrap()
                                    )
                                })
                                .unwrap_or_default();
                            let delta = delta.map(js::number_to_string).unwrap_or_default();
                            format!("{name}{change}{delta}")
                        }
                    })
                    .collect();
                (strip.state, tiles.join(" "))
            })
            .collect()
    }

    #[test]
    fn timeline_strips_cases() {
        use TimelineStripState::{Changed, New};
        let moved: Vec<Clip> = row(10)
            .into_iter()
            .map(|c| {
                if c.id == "c5" {
                    Clip {
                        start: c.start + 48.0,
                        end: c.end + 48.0,
                        ..c
                    }
                } else {
                    c
                }
            })
            .collect();
        let first24: Vec<String> = row(24).into_iter().map(|c| c.id).collect();
        let four = row(4);
        let three = row(3);
        let cases: Vec<Case<(TimelineStripState, String)>> = vec![
            (
                "a new timeline shows its whole cut",
                vec![],
                vec![with_cuts(row(3))],
                vec![(New, "c0 c1 c2".into())],
            ),
            (
                "a long new timeline folds past the limit",
                vec![],
                vec![with_cuts(row(30))],
                vec![(New, format!("{} +6", first24.join(" ")))],
            ),
            (
                "a move keeps a neighbour each side and folds the rest",
                vec![with_cuts(row(10))],
                vec![with_cuts(moved)],
                vec![(Changed, "+5 c6 c5:moved2 c7 +2".into())],
            ),
            (
                "a trim shows the change in length, a removal shows where the clip was",
                vec![with_cuts(four.clone())],
                vec![with_cuts(vec![
                    Clip {
                        end: 12.0,
                        ..four[0].clone()
                    },
                    four[1].clone(),
                    four[2].clone(),
                ])],
                vec![(Changed, "c0:trimmed-0.5 c1 c2 c3:removed".into())],
            ),
            (
                "a lone untouched clip is not folded",
                vec![with_cuts(three.clone())],
                vec![with_cuts(vec![
                    three[0].clone(),
                    three[1].clone(),
                    Clip {
                        name: "renamed".into(),
                        ..three[2].clone()
                    },
                ])],
                vec![(Changed, "c0 c1 renamed:changed".into())],
            ),
            (
                "no strip when the clips did not change",
                vec![with_cuts(row(3))],
                vec![StoredTimeline {
                    markers: vec![marker(1.0, "Blue", "")],
                    ..with_cuts(row(3))
                }],
                vec![],
            ),
        ];
        for (name, before, after, expected) in cases {
            assert_eq!(short(&before, &after), expected, "{name}");
        }
    }

    #[test]
    fn marks_stills_so_the_panel_can_show_them() {
        let strips = timeline_strips(&[], &[with_cuts(row(4))], is_image);
        let images: Vec<bool> = strips[0]
            .tiles
            .iter()
            .map(|tile| matches!(tile, StripTile::Clip { image: true, .. }))
            .collect();
        assert_eq!(images, [false, false, false, true]);
    }

    #[test]
    fn timeline_folders_are_safe_on_every_os_and_unique() {
        assert_eq!(
            timeline_folders(&["AI/Rough cut", "AI_Rough cut", "a:b?", "Trailing. ", "///"]),
            [
                "AI_Rough cut",
                "AI_Rough cut (2)",
                "a_b_",
                "Trailing",
                "___"
            ]
        );
    }
}
