//! Port of src/lib/player.ts: which clip shows when, the edit points, the timecode, and a clip's
//! look (Resolve's transform) as the transform the picture gets.

use std::collections::BTreeMap;

use linker_edit_protocol::{LookValue, PlayableClip, PlayableTimeline};

/// What the preview panel shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewTarget {
    /// None for Resolve's current timeline.
    pub id: Option<String>,
    /// A save from the history; None for the timeline as it is in Resolve now.
    pub commit: Option<String>,
}

/// "0:06:12" (minutes, seconds, frames), with hours once a timeline runs that long.
pub fn format_timecode(seconds: f64, fps: f64) -> String {
    let rate = match fps.round() as i64 {
        0 => 24,
        rate => rate,
    };
    let total_frames = (seconds * rate as f64).round().max(0.) as i64;
    let frames = total_frames % rate;
    let whole = total_frames / rate;
    let (h, m) = (whole / 3600, (whole % 3600) / 60);
    let minutes = if h > 0 {
        format!("{h}:{m:02}")
    } else {
        m.to_string()
    };
    format!("{minutes}:{:02}:{frames:02}", whole % 60)
}

/// The clip showing on each track at a time, bottom track first.
pub fn clips_at(clips: &[PlayableClip], time: f64) -> Vec<&PlayableClip> {
    let mut by_track: BTreeMap<u32, &PlayableClip> = BTreeMap::new();
    for clip in clips {
        if clip.start <= time && time < clip.end {
            by_track.insert(clip.track, clip);
        }
    }
    by_track.into_values().collect()
}

/// How far ahead clips are loaded, so the cut to them doesn't stall.
pub const AHEAD: f64 = 3.;

/// Clips starting within `seconds`, loaded ahead so the cut to them doesn't stall.
pub fn clips_ahead(clips: &[PlayableClip], time: f64, seconds: f64) -> Vec<&PlayableClip> {
    clips
        .iter()
        .filter(|clip| clip.start > time && clip.start <= time + seconds)
        .collect()
}

/// The clip's key among a timeline's clips: its track and start, as `clipKey`.
pub fn clip_key(clip: &PlayableClip) -> String {
    format!("{}:{}", clip.track, clip.start)
}

/// The previous (-1) or next (1) edit point, including the timeline's start and end.
pub fn nearest_cut(timeline: &PlayableTimeline, time: f64, direction: i8) -> f64 {
    let mut cuts = vec![0., timeline.duration];
    cuts.extend(
        timeline
            .video
            .iter()
            .flat_map(|clip| [clip.start, clip.end]),
    );
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    // Half a frame of slack, so a playhead parked on a cut moves on to the next one.
    let slack = 0.5 / timeline.fps;
    if direction > 0 {
        cuts.into_iter()
            .find(|&cut| cut > time + slack)
            .unwrap_or(timeline.duration)
    } else {
        cuts.into_iter()
            .rev()
            .find(|&cut| cut < time - slack)
            .unwrap_or(0.)
    }
}

/// A clip's look as lookStyle's CSS on a full-frame element: translate, then rotate, then scale,
/// about the frame's center, and an opacity.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    /// Across, as a fraction of the frame's width (CSS `translate(x%)` / 100).
    pub x: f64,
    /// Down, as a fraction of the frame's height.
    pub y: f64,
    /// Degrees, clockwise, as CSS `rotate()`.
    pub rotate: f64,
    /// Negative mirrors.
    pub scale_x: f64,
    pub scale_y: f64,
    pub opacity: f64,
}

impl Transform {
    pub const IDENTITY: Transform = Transform {
        x: 0.,
        y: 0.,
        rotate: 0.,
        scale_x: 1.,
        scale_y: 1.,
        opacity: 1.,
    };

    /// Resolve's transform: Pan and Tilt are in timeline pixels, Tilt points up and positive
    /// rotation turns counter-clockwise.
    pub fn of(look: &BTreeMap<String, LookValue>, width: u32, height: u32) -> Transform {
        let number = |key: &str, fallback: f64| match look.get(key) {
            Some(LookValue::Number(value)) => *value,
            _ => fallback,
        };
        // JavaScript's truthiness: true, or a number other than 0.
        let on = |key: &str| match look.get(key) {
            Some(LookValue::Bool(value)) => *value,
            Some(LookValue::Number(value)) => *value != 0. && !value.is_nan(),
            None => false,
        };
        let flip = |key: &str| if on(key) { -1. } else { 1. };
        Transform {
            x: number("Pan", 0.) / width.max(1) as f64,
            y: -number("Tilt", 0.) / height.max(1) as f64,
            rotate: -number("RotationAngle", 0.),
            scale_x: number("ZoomX", 1.) * flip("FlipX"),
            scale_y: number("ZoomY", 1.) * flip("FlipY"),
            opacity: number("Opacity", 100.) / 100.,
        }
    }

    /// Whether the picture is where object-contain puts it, unchanged (opacity aside).
    pub fn moves_picture(&self) -> bool {
        Transform {
            opacity: 1.,
            ..*self
        } != Transform::IDENTITY
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use linker_edit_protocol::MediaKind;

    fn clip(name: &str, start: f64, end: f64, track: u32) -> PlayableClip {
        PlayableClip {
            name: name.into(),
            track,
            start,
            end,
            offset: 0.,
            rate: 1.,
            media: Some(name.into()),
            kind: MediaKind::Video,
            look: BTreeMap::new(),
        }
    }

    // a [0–2) b [2–5) on V1, a gap until 6, c [6–8); a title on V2 over [1–3).
    fn video() -> Vec<PlayableClip> {
        vec![
            clip("a", 0., 2., 1),
            clip("title", 1., 3., 2),
            clip("b", 2., 5., 1),
            clip("c", 6., 8., 1),
        ]
    }

    fn names(clips: Vec<&PlayableClip>) -> Vec<&str> {
        clips.into_iter().map(|clip| clip.name.as_str()).collect()
    }

    #[test]
    fn shows_the_top_of_each_track() {
        let video = video();
        for (time, expected) in [
            (0., vec!["a"]),
            (1.5, vec!["a", "title"]),
            (2., vec!["b", "title"]),
            (5.5, vec![]),
            (8., vec![]),
        ] {
            assert_eq!(names(clips_at(&video, time)), expected, "at {time}s");
        }
    }

    #[test]
    fn loads_the_clips_starting_in_the_next_few_seconds() {
        let video = video();
        assert_eq!(names(clips_ahead(&video, 1.5, AHEAD)), ["b"]);
        assert_eq!(names(clips_ahead(&video, 4., AHEAD)), ["c"]);
    }

    #[test]
    fn finds_the_cut_in_each_direction() {
        let timeline = PlayableTimeline {
            id: "t".into(),
            name: "T".into(),
            commit: None,
            fps: 24.,
            width: 1920,
            height: 1080,
            duration: 10.,
            video: video(),
            audio: vec![],
            markers: vec![],
            replaces: None,
        };
        for (time, direction, expected) in [
            (0., 1, 1.),
            (1., 1, 2.),
            (2.01, 1, 3.),
            (5.5, 1, 6.),
            (8., 1, 10.),
            (10., 1, 10.),
            (2., -1, 1.),
            (2.01, -1, 1.),
            (0.5, -1, 0.),
        ] {
            assert_eq!(
                nearest_cut(&timeline, time, direction),
                expected,
                "from {time}s in direction {direction}"
            );
        }
    }

    #[test]
    fn writes_timecodes() {
        for (seconds, expected) in [
            (0., "0:00:00"),
            (6.5, "0:06:12"),
            (61. + 23. / 24., "1:01:23"),
            (3600. * 2. + 5., "2:00:05:00"),
        ] {
            assert_eq!(format_timecode(seconds, 24.), expected);
        }
    }

    #[test]
    fn turns_resolves_transform_into_the_pictures() {
        let look: BTreeMap<String, LookValue> = [
            ("ZoomX", LookValue::Number(1.3)),
            ("ZoomY", LookValue::Number(1.3)),
            ("Pan", LookValue::Number(192.)),
            ("Tilt", LookValue::Number(108.)),
            ("RotationAngle", LookValue::Number(90.)),
            ("Opacity", LookValue::Number(50.)),
            ("FlipX", LookValue::Bool(true)),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_string(), value))
        .collect();
        // lookStyle: "translate(10%, -10%) rotate(-90deg) scale(-1.3, 1.3); opacity: 0.5"
        assert_eq!(
            Transform::of(&look, 1920, 1080),
            Transform {
                x: 0.1,
                y: -0.1,
                rotate: -90.,
                scale_x: -1.3,
                scale_y: 1.3,
                opacity: 0.5,
            }
        );
        assert_eq!(
            Transform::of(&BTreeMap::new(), 1920, 1080),
            Transform::IDENTITY
        );
        let faded = BTreeMap::from([("Opacity".to_string(), LookValue::Number(40.))]);
        assert!(!Transform::of(&faded, 1920, 1080).moves_picture());
    }
}
