// A saved or live timeline as the browser's quick preview plays it, straight from the media files.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use linker_edit_protocol::{LookValue, MediaKind, PlayableClip, PlayableTimeline, TimelineMarker};
use serde_json::Value;

use super::model::{Clip, MediaEntry, StoredTimeline};
use crate::js;

/// What kind of file the browser can show or play, and its content type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaType {
    pub kind: MediaKind,
    pub r#type: &'static str,
}

// Files a browser can show or play, by extension. A .mov may still hold a codec browsers can't
// decode, such as ProRes; the player shows those as unplayable when they fail to load.
const MEDIA_TYPES: [(&str, MediaKind, &str); 17] = [
    (".jpg", MediaKind::Image, "image/jpeg"),
    (".jpeg", MediaKind::Image, "image/jpeg"),
    (".png", MediaKind::Image, "image/png"),
    (".webp", MediaKind::Image, "image/webp"),
    (".gif", MediaKind::Image, "image/gif"),
    (".bmp", MediaKind::Image, "image/bmp"),
    (".avif", MediaKind::Image, "image/avif"),
    (".mp4", MediaKind::Video, "video/mp4"),
    (".m4v", MediaKind::Video, "video/mp4"),
    (".mov", MediaKind::Video, "video/quicktime"),
    (".webm", MediaKind::Video, "video/webm"),
    (".mp3", MediaKind::Audio, "audio/mpeg"),
    (".m4a", MediaKind::Audio, "audio/mp4"),
    (".aac", MediaKind::Audio, "audio/aac"),
    (".wav", MediaKind::Audio, "audio/wav"),
    (".ogg", MediaKind::Audio, "audio/ogg"),
    (".flac", MediaKind::Audio, "audio/flac"),
];

/// path.extname, also for a Windows path read on another OS: "D:\footage\a.MP4" is ".MP4".
fn extname(file: &str) -> String {
    let base = file.rsplit(['/', '\\']).next().unwrap_or(file);
    match base.rfind('.') {
        Some(0) | None => String::new(),
        Some(dot) => base[dot..].to_string(),
    }
}

pub fn media_type(file: Option<&str>) -> Option<MediaType> {
    let ext = extname(file?).to_lowercase();
    MEDIA_TYPES
        .iter()
        .find(|(candidate, _, _)| *candidate == ext)
        .map(|&(_, kind, r#type)| MediaType { kind, r#type })
}

/// media_type for a path on this computer.
pub fn media_type_of(file: &Path) -> Option<MediaType> {
    media_type(file.to_str())
}

// The properties the quick preview can approximate with CSS.
const LOOK: [&str; 8] = [
    "ZoomX",
    "ZoomY",
    "Pan",
    "Tilt",
    "RotationAngle",
    "Opacity",
    "FlipX",
    "FlipY",
];

pub fn playable_timeline(
    timeline: &StoredTimeline,
    media: &HashMap<String, MediaEntry>,
    commit: Option<String>,
) -> PlayableTimeline {
    let metadata = &timeline.metadata;
    let fps = if metadata.frame_rate == 0.0 || metadata.frame_rate.is_nan() {
        24.0
    } else {
        metadata.frame_rate
    };
    let seconds = |frame: f64| (frame - metadata.start_frame) / fps;

    let clip = |source: &Clip| -> PlayableClip {
        let entry = source.media.as_ref().and_then(|id| media.get(id));
        let file = entry.map(|entry| entry.path.as_str());
        let empty = serde_json::Map::new();
        let properties = source.properties.as_ref().unwrap_or(&empty);
        let look: BTreeMap<String, LookValue> = LOOK
            .iter()
            .filter_map(|key| {
                let value = match properties.get(*key)? {
                    Value::Bool(value) => LookValue::Bool(*value),
                    value => LookValue::Number(value.as_f64()?),
                };
                Some((key.to_string(), value))
            })
            .collect();
        let source_fps = entry
            .and_then(|entry| entry.fps)
            .filter(|fps| *fps != 0.0 && !fps.is_nan());
        let speed = match properties.get("Speed") {
            None | Some(Value::Null) => 100.0,
            speed => js::number(speed),
        };
        let rate = speed / 100.0;
        PlayableClip {
            name: source.name.clone(),
            track: source.track as u32,
            start: seconds(source.start),
            end: seconds(source.end),
            // Source frames count at the media's own rate: 25 fps footage on a 24 fps timeline is common.
            offset: source.source_start.unwrap_or(0.0).max(0.0) / source_fps.unwrap_or(fps),
            rate: if rate == 0.0 || rate.is_nan() {
                1.0
            } else {
                rate
            },
            media: file.and(source.media.clone()),
            kind: match &source.media {
                None => MediaKind::Missing,
                Some(_) => media_type(file)
                    .map(|found| found.kind)
                    .unwrap_or(if file.is_some() {
                        MediaKind::Unsupported
                    } else {
                        MediaKind::Missing
                    }),
            },
            look,
        }
    };
    let playable = |clips: &[Clip]| -> Vec<PlayableClip> {
        let mut played: Vec<PlayableClip> = clips
            .iter()
            .filter(|source| source.enabled != Some(false))
            .map(clip)
            .collect();
        played.sort_by(|a, b| {
            a.start
                .partial_cmp(&b.start)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.track.cmp(&b.track))
        });
        played
    };

    let dimension = |value: f64, fallback: u32| {
        if value == 0.0 || value.is_nan() {
            fallback
        } else {
            value as u32
        }
    };
    PlayableTimeline {
        id: metadata.id.clone(),
        name: metadata.name.clone(),
        commit,
        fps,
        width: dimension(metadata.width, 1920),
        height: dimension(metadata.height, 1080),
        duration: ((metadata.end_frame - metadata.start_frame) / fps).max(0.0),
        video: playable(&timeline.cuts),
        audio: playable(&timeline.audio),
        markers: timeline
            .markers
            .iter()
            .map(|marker| TimelineMarker {
                time: marker.frame / fps,
                color: marker.color.clone(),
                name: marker.name.clone(),
            })
            .collect(),
        replaces: None,
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::model::{Marker, TimelineMetadata};
    use super::*;

    fn clip(id: &str) -> Clip {
        Clip {
            track: 1.0,
            id: id.into(),
            name: id.into(),
            start: 86400.0,
            end: 86448.0,
            source_start: Some(0.0),
            source_end: Some(48.0),
            media: Some(format!("m-{id}")),
            enabled: None,
            fusion_comps: None,
            properties: None,
        }
    }

    fn entry(id: &str, path: &str, fps: Option<f64>) -> (String, MediaEntry) {
        let entry = MediaEntry {
            id: id.into(),
            name: String::new(),
            path: path.into(),
            fps,
        };
        (id.into(), entry)
    }

    #[test]
    fn places_clips_in_seconds_from_the_timeline_start_with_what_the_browser_can_play() {
        let media: HashMap<String, MediaEntry> = [
            entry("m-a", "D:\\footage\\a.MP4", Some(25.0)),
            entry("m-b", "D:\\stills\\b.jpg", None),
            entry("m-c", "D:\\footage\\c.braw", None),
            entry("m-v", "D:\\audio\\v.wav", None),
        ]
        .into_iter()
        .collect();
        let properties = |value: Value| value.as_object().cloned();
        let timeline = StoredTimeline {
            metadata: TimelineMetadata {
                id: "t1".into(),
                name: "AI - Cut".into(),
                frame_rate: 24.0,
                width: 1920.0,
                height: 1080.0,
                start_frame: 86400.0,
                end_frame: 86400.0 + 240.0,
                start_timecode: "01:00:00:00".into(),
                video_tracks: vec!["V1".into(), "V2".into()],
                audio_tracks: vec!["A1".into()],
            },
            cuts: vec![
                Clip {
                    start: 86448.0,
                    end: 86496.0,
                    properties: properties(json!({ "ZoomX": 1.3, "ZoomY": 1.3, "CropLeft": 10 })),
                    ..clip("b")
                },
                // a's source runs at 25 fps, so its frame 125 is 5 s in.
                Clip {
                    source_start: Some(125.0),
                    properties: properties(json!({ "Speed": 200 })),
                    ..clip("a")
                },
                Clip {
                    start: 86496.0,
                    end: 86544.0,
                    ..clip("c")
                },
                Clip {
                    start: 86544.0,
                    end: 86592.0,
                    ..clip("gone")
                },
                Clip {
                    enabled: Some(false),
                    ..clip("off")
                },
                Clip {
                    track: 2.0,
                    media: None,
                    ..clip("title")
                },
            ],
            audio: vec![Clip {
                end: 86640.0,
                ..clip("v")
            }],
            markers: vec![Marker {
                frame: 36.0,
                color: "Red".into(),
                name: "Drop".into(),
                note: String::new(),
                duration: 1.0,
            }],
        };
        let played = playable_timeline(&timeline, &media, Some("f8e032f".into()));
        assert_eq!(played.duration, 10.0);
        assert_eq!(played.commit.as_deref(), Some("f8e032f"));
        assert_eq!(
            played.markers,
            [TimelineMarker {
                time: 1.5,
                color: "Red".into(),
                name: "Drop".into()
            }]
        );
        let video: Vec<Value> = played
            .video
            .iter()
            .map(|c| {
                json!([
                    c.name, c.start, c.end, c.offset, c.rate, c.kind, c.media, c.look
                ])
            })
            .collect();
        assert_eq!(
            video,
            [
                json!(["a", 0.0, 2.0, 5.0, 2.0, "video", "m-a", {}]),
                json!(["title", 0.0, 2.0, 0.0, 1.0, "missing", null, {}]),
                json!(["b", 2.0, 4.0, 0.0, 1.0, "image", "m-b", { "ZoomX": 1.3, "ZoomY": 1.3 }]),
                json!(["c", 4.0, 6.0, 0.0, 1.0, "unsupported", "m-c", {}]),
                json!(["gone", 6.0, 8.0, 0.0, 1.0, "missing", null, {}]),
            ]
        );
        assert_eq!(played.audio.len(), 1);
        let audio = &played.audio[0];
        assert_eq!(
            (audio.name.as_str(), audio.start, audio.end, audio.kind),
            ("v", 0.0, 10.0, MediaKind::Audio)
        );
    }
}
