//! The playback engine both preview views use: a timeline's clips (or one rendered file) played
//! in sync, as TimelinePlayer.svelte's media elements follow its clock.
//!
//! The clock is the sound card's ([`audio::Mixer`]); without one, the wall clock. The window
//! calls [`Player::tick`] each frame it draws: it keeps one decoder per clip on screen or
//! coming up within [`AHEAD`] seconds, takes the frame each is due to show, and hands back what
//! to draw. Paused, each move of the playhead decodes the one frame there (a still).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{App, RenderImage, Window};
use linker_edit_protocol::{MediaKind, PlayableClip, PlayableTimeline};

use super::audio::{Output, Source, Span};
use super::timeline::{AHEAD, Transform, clip_key, clips_ahead, clips_at};
use super::video::{Decode, Fit, Timing, VideoFrame, VideoStream};
use crate::state::ServerAddress;

/// Playing starts once every decoder has its first frame and samples, or after this.
const START_WAIT: Duration = Duration::from_millis(1500);

/// What plays: a timeline's clips, or one rendered file.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub timeline: PlayableTimeline,
    /// Each clip's media is a URL path on the server (a render); else a media id at /media/<id>.
    pub direct: bool,
}

impl Program {
    /// Cut preview: the timeline's own media.
    pub fn cut(timeline: PlayableTimeline) -> Program {
        Program {
            timeline,
            direct: false,
        }
    }

    /// Full render: the file at `path` on the server, as one clip with its sound.
    pub fn render(path: &str, timeline: &PlayableTimeline, duration: f64) -> Program {
        let clip = PlayableClip {
            name: timeline.name.clone(),
            track: 1,
            start: 0.,
            end: duration,
            offset: 0.,
            rate: 1.,
            media: Some(path.to_string()),
            kind: MediaKind::Video,
            look: Default::default(),
        };
        Program {
            timeline: PlayableTimeline {
                duration,
                video: vec![clip.clone()],
                audio: vec![clip],
                markers: Vec::new(),
                replaces: None,
                ..timeline.clone()
            },
            direct: true,
        }
    }
}

/// A clip on screen, for the timeline player to draw.
#[derive(Clone)]
pub struct Picture {
    pub clip: PlayableClip,
    /// False when it can't play: no media, missing or unsupported, or ffmpeg gave up on it.
    pub playable: bool,
    /// Its frame now, the size of the stage; None until the first is decoded.
    pub image: Option<Arc<RenderImage>>,
    pub opacity: f32,
}

#[derive(Default)]
struct Layer {
    stream: Option<VideoStream>,
    shown: Option<VideoFrame>,
    /// Where the last still was asked for.
    still_at: Option<f64>,
}

/// The wall clock, for when there is no sound card.
#[derive(Default)]
struct Wall {
    base: f64,
    since: Option<Instant>,
}

impl Wall {
    fn time(&self) -> f64 {
        self.base + self.since.map_or(0., |since| since.elapsed().as_secs_f64())
    }
}

pub struct Player {
    server: Option<ServerAddress>,
    program: Option<Program>,
    output: Option<Output>,
    wall: Wall,
    playing: bool,
    /// Play was pressed and the decoders are filling; the clock waits for them.
    starting: Option<Instant>,
    /// Device pixels.
    stage: (u32, u32),
    layers: HashMap<String, Layer>,
    thumbnails: HashMap<String, Layer>,
    /// Media ffmpeg couldn't play, as TimelinePlayer's `failed`.
    failed: HashSet<String>,
}

impl Player {
    /// A player for the server's media, on the sound card if there is one. Tests never touch
    /// the sound card.
    pub fn new(server: Option<ServerAddress>) -> Player {
        if cfg!(test) {
            return Player::silent(server);
        }
        let output = match Output::open() {
            Ok(output) => Some(output),
            Err(error) => {
                eprintln!("No sound: {error}. The preview plays without it.");
                None
            }
        };
        Player::with_output(server, output)
    }

    /// Without a sound card: the wall clock (tests, and computers with none).
    pub fn silent(server: Option<ServerAddress>) -> Player {
        Player::with_output(server, None)
    }

    fn with_output(server: Option<ServerAddress>, output: Option<Output>) -> Player {
        Player {
            server,
            program: None,
            output,
            wall: Wall::default(),
            playing: false,
            starting: None,
            stage: (0, 0),
            layers: HashMap::new(),
            thumbnails: HashMap::new(),
            failed: HashSet::new(),
        }
    }

    pub fn has_sound(&self) -> bool {
        self.output.is_some()
    }

    pub fn program(&self) -> Option<&Program> {
        self.program.as_ref()
    }

    pub fn duration(&self) -> f64 {
        self.program
            .as_ref()
            .map_or(0., |program| program.timeline.duration)
    }

    pub fn playing(&self) -> bool {
        self.playing
    }

    /// Where the playhead is.
    pub fn time(&self) -> f64 {
        let time = match &self.output {
            Some(output) => output.mixer.time(),
            None => self.wall.time(),
        };
        time.min(self.duration())
    }

    /// Plays `program` from where the playhead is; the same program again changes nothing.
    pub fn set_program(&mut self, program: Option<Program>, window: &mut Window, cx: &mut App) {
        if self.program == program {
            return;
        }
        let time = self.time();
        self.program = program;
        self.failed.clear();
        self.clear(window, cx);
        self.seek(time);
    }

    pub fn seek(&mut self, time: f64) {
        let time = time.clamp(0., self.duration());
        self.set_clock(time);
        for layer in self.layers.values_mut() {
            layer.stream = None;
        }
        if self.playing {
            self.starting = Some(Instant::now());
            self.run_clock(false);
        }
    }

    pub fn play(&mut self) {
        if self.playing {
            return;
        }
        self.playing = true;
        self.starting = Some(Instant::now());
        for layer in self.layers.values_mut() {
            if layer.stream.as_ref().is_some_and(VideoStream::still) {
                layer.stream = None;
            }
        }
    }

    pub fn pause(&mut self) {
        let time = self.time();
        self.playing = false;
        self.starting = None;
        self.run_clock(false);
        self.set_clock_keep_sources(time);
    }

    pub fn set_volume(&mut self, muted: bool, volume: f32) {
        if let Some(output) = &self.output {
            output.mixer.set_gain(if muted { 0. } else { volume });
        }
    }

    /// The stage's size in device pixels; frames are decoded at it.
    pub fn set_stage(&mut self, size: (u32, u32)) {
        if size != self.stage {
            self.stage = size;
            for layer in self.layers.values_mut() {
                layer.stream = None;
                layer.still_at = None;
            }
        }
    }

    /// When the window should draw again: when the next frame is due while playing, soon while
    /// waiting on a decoder, None when nothing will change.
    pub fn next_draw(&mut self) -> Option<Duration> {
        const POLL: f64 = 0.01;
        let period = 1.
            / self
                .program
                .as_ref()
                .map_or(30., |program| program.timeline.fps.max(1.));
        // A still in flight (or its ffmpeg not gone yet), or a stream without its first frame.
        let waiting = self.layers.values_mut().any(|Layer { stream, shown, .. }| {
            stream
                .as_mut()
                .is_some_and(|stream| stream.still() || (!stream.done() && shown.is_none()))
        }) || self.thumbnails.values().any(|thumb| thumb.stream.is_some());
        let seconds = if self.playing && self.starting.is_none() {
            let time = self.time();
            let due = self
                .layers
                .values_mut()
                .filter_map(|layer| layer.stream.as_mut()?.next_time())
                .map(|at| at - time)
                .fold(period, f64::min);
            due.max(0.002)
        } else if self.playing || waiting {
            POLL
        } else {
            return None;
        };
        Some(Duration::from_secs_f64(seconds))
    }

    /// Stops decoding and lets go of the frames (when the panel closes).
    pub fn stop(&mut self, window: &mut Window, cx: &mut App) {
        self.pause();
        self.clear(window, cx);
        for (_, thumb) in self.thumbnails.drain() {
            if let Some(shown) = thumb.shown {
                cx.drop_image(shown.image, Some(window));
            }
        }
    }

    fn clear(&mut self, window: &mut Window, cx: &mut App) {
        for (_, layer) in self.layers.drain() {
            if let Some(shown) = layer.shown {
                cx.drop_image(shown.image, Some(window));
            }
        }
        if let Some(output) = &self.output {
            output.mixer.retain(|_| false);
        }
    }

    fn set_clock(&mut self, time: f64) {
        if let Some(output) = &self.output {
            output.mixer.seek(time);
        }
        self.set_clock_keep_sources(time);
    }

    fn set_clock_keep_sources(&mut self, time: f64) {
        self.wall = Wall {
            base: time,
            since: self.wall.since.map(|_| Instant::now()),
        };
    }

    fn run_clock(&mut self, running: bool) {
        if let Some(output) = &self.output {
            output.mixer.set_playing(running);
        }
        let time = self.wall.time();
        self.wall = Wall {
            base: time,
            since: running.then(Instant::now),
        };
    }

    fn url(&self, media: &str) -> Option<String> {
        let server = self.server.as_ref()?;
        let direct = self.program.as_ref().is_some_and(|program| program.direct);
        Some(if direct {
            server.url(media)
        } else {
            server.url(&format!("/media/{}", encode_component(media)))
        })
    }

    fn playable(&self, clip: &PlayableClip) -> bool {
        clip.media
            .as_ref()
            .is_some_and(|media| !self.failed.contains(media))
            && !matches!(clip.kind, MediaKind::Missing | MediaKind::Unsupported)
    }

    /// Moves the clock on, keeps the decoders in step with it, and says what to draw now.
    pub fn tick(&mut self, window: &mut Window, cx: &mut App) -> Vec<Picture> {
        let Some(program) = self.program.clone() else {
            return Vec::new();
        };
        let timeline = &program.timeline;
        if self
            .output
            .as_ref()
            .is_some_and(|output| output.mixer.broken())
        {
            // The sound card went away: keep going on the wall clock.
            let time = self.time();
            self.output = None;
            self.wall = Wall {
                base: time,
                since: (self.playing && self.starting.is_none()).then(Instant::now),
            };
        }
        let mut time = self.time();
        if self.playing && self.starting.is_none() && time >= timeline.duration {
            self.pause();
            self.set_clock_keep_sources(timeline.duration);
            time = timeline.duration;
        }

        let now = clips_at(&timeline.video, time);
        let ahead = if self.playing {
            clips_ahead(&timeline.video, time, AHEAD)
        } else {
            Vec::new()
        };
        let wanted: HashSet<String> = now
            .iter()
            .chain(&ahead)
            .map(|clip| clip_key(clip))
            .collect();
        let gone: Vec<String> = self
            .layers
            .keys()
            .filter(|key| !wanted.contains(*key))
            .cloned()
            .collect();
        for key in gone {
            if let Some(shown) = self.layers.remove(&key).and_then(|layer| layer.shown) {
                cx.drop_image(shown.image, Some(window));
            }
        }

        let fps = timeline.fps.max(1.);
        for clip in now.iter().chain(&ahead) {
            if !self.playable(clip) || self.stage.0 == 0 {
                continue;
            }
            // What the decoder has, first: a still that just came may be for an older playhead.
            let layer = self.layers.entry(clip_key(clip)).or_default();
            if let Some(stream) = &mut layer.stream {
                if let Some(frame) = stream.take_due(time)
                    && let Some(old) = layer.shown.replace(frame)
                {
                    cx.drop_image(old.image, Some(window));
                }
                match stream.finished() {
                    Some(false) => {
                        layer.stream = None;
                        self.failed.extend(clip.media.clone());
                        continue;
                    }
                    Some(true) if stream.still() => layer.stream = None,
                    _ => {}
                }
            }
            if let Some(decode) = self.decode_for(clip, time, fps, &program) {
                let server = self.server.as_ref().expect("decode_for needs a server");
                let layer = self.layers.entry(clip_key(clip)).or_default();
                match VideoStream::start(server, decode) {
                    Ok(stream) => layer.stream = Some(stream),
                    Err(error) => {
                        eprintln!("Couldn't play {}: {error}", clip.name);
                        self.failed.extend(clip.media.clone());
                    }
                }
            }
        }

        self.keep_sound(timeline, time);

        if let Some(started) = self.starting {
            let ready = now.iter().all(|clip| {
                self.layers
                    .get_mut(&clip_key(clip))
                    .and_then(|layer| layer.stream.as_mut())
                    .is_none_or(VideoStream::ready)
            }) && self
                .output
                .as_ref()
                .is_none_or(|output| output.mixer.ready());
            if ready || started.elapsed() > START_WAIT {
                self.starting = None;
                self.run_clock(true);
            }
        }

        now.into_iter()
            .map(|clip| Picture {
                clip: clip.clone(),
                playable: self.playable(clip),
                image: self
                    .layers
                    .get(&clip_key(clip))
                    .and_then(|layer| layer.shown.as_ref())
                    .map(|frame| frame.image.clone()),
                opacity: Transform::of(&clip.look, timeline.width, timeline.height).opacity as f32,
            })
            .collect()
    }

    /// The decoder `clip`'s layer needs now, if it needs a new one.
    fn decode_for(
        &mut self,
        clip: &PlayableClip,
        time: f64,
        fps: f64,
        program: &Program,
    ) -> Option<Decode> {
        let url = self.url(clip.media.as_deref()?)?;
        let timeline = &program.timeline;
        let transform = Transform::of(&clip.look, timeline.width, timeline.height);
        let layer = self.layers.entry(clip_key(clip)).or_default();
        // ffmpeg starts at the first frame at or after the seek; half a frame early gets the frame
        // on screen at `at`, as a video element shows, whatever rounding `at` has.
        let still = |at: f64| Decode {
            url: url.clone(),
            seek: Some(clip.offset + (at - clip.start) * clip.rate - 0.5 / fps),
            timing: None,
            first: at,
            length: None,
            size: self.stage,
            transform,
            fit: Fit::Contain,
        };
        if clip.kind == MediaKind::Image {
            // One frame for as long as the clip lasts.
            if layer.still_at.is_some() {
                return None;
            }
            layer.still_at = Some(time);
            return Some(Decode {
                seek: None,
                ..still(time)
            });
        }
        if self.playing {
            if layer.stream.as_ref().is_none_or(VideoStream::still) {
                let from = time.max(clip.start);
                return Some(Decode {
                    timing: Some(Timing {
                        fps,
                        rate: clip.rate,
                    }),
                    length: Some(clip.end - from),
                    ..still(from)
                });
            }
            return None;
        }
        // Paused: the frame at the playhead, one at a time while it moves.
        let close = |at: Option<f64>, slack: f64| at.is_some_and(|at| (at - time).abs() < slack);
        let shown = layer.shown.as_ref().map(|frame| frame.time);
        let decoding = layer
            .stream
            .as_mut()
            .is_some_and(|stream| stream.still() && !stream.done());
        if decoding || close(shown, 0.5 / fps) || close(layer.still_at, 0.5 / fps) {
            return None;
        }
        layer.still_at = Some(time);
        Some(still(time))
    }

    /// One sound source per audible clip playing now or soon; the rest stop.
    fn keep_sound(&mut self, timeline: &PlayableTimeline, time: f64) {
        let (Some(output), Some(server)) = (&self.output, &self.server) else {
            return;
        };
        let audible: Vec<PlayableClip> = timeline
            .audio
            .iter()
            .filter(|clip| {
                self.playable(clip) && matches!(clip.kind, MediaKind::Audio | MediaKind::Video)
            })
            .cloned()
            .collect();
        let mut due: Vec<&PlayableClip> = audible
            .iter()
            .filter(|clip| clip.start <= time && time < clip.end)
            .collect();
        if self.playing {
            due.extend(clips_ahead(&audible, time, AHEAD));
        }
        let keys: HashSet<String> = due
            .iter()
            .map(|clip| format!("a{}", clip_key(clip)))
            .collect();
        output.mixer.retain(|key| keys.contains(key));
        if !self.playing {
            return;
        }
        for clip in due {
            let key = format!("a{}", clip_key(clip));
            if output.mixer.has(&key) {
                continue;
            }
            let Some(url) = self.url(clip.media.as_deref().unwrap_or_default()) else {
                continue;
            };
            let from = time.max(clip.start);
            let span = Span {
                seek: clip.offset + (from - clip.start) * clip.rate,
                length: clip.end - from,
                speed: clip.rate,
                first: output.mixer.sample(from),
            };
            match Source::decode(server, key, &url, span, output.mixer.rate) {
                Ok(source) => output.mixer.add(source),
                Err(error) => eprintln!("Couldn't play the sound of {}: {error}", clip.name),
            }
        }
    }

    /// A small picture of an image clip for the scrubber, as `background: center / cover`.
    pub fn thumbnail(
        &mut self,
        media: &str,
        size: (u32, u32),
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<RenderImage>> {
        let server = self.server.clone()?;
        // The timeline's media, also while a render plays.
        let url = server.url(&format!("/media/{}", encode_component(media)));
        let thumb = self.thumbnails.entry(media.to_string()).or_default();
        if thumb.shown.is_none() && thumb.stream.is_none() && thumb.still_at.is_none() {
            thumb.still_at = Some(0.);
            thumb.stream = VideoStream::start(
                &server,
                Decode {
                    url,
                    seek: None,
                    timing: None,
                    first: 0.,
                    length: None,
                    size,
                    transform: Transform::IDENTITY,
                    fit: Fit::Cover,
                },
            )
            .ok();
        }
        if let Some(stream) = &mut thumb.stream {
            if let Some(frame) = stream.take_due(0.)
                && let Some(old) = thumb.shown.replace(frame)
            {
                cx.drop_image(old.image, Some(window));
            }
            if stream.done() {
                thumb.stream = None;
            }
        }
        thumb.shown.as_ref().map(|frame| frame.image.clone())
    }
}

/// encodeURIComponent: everything but letters, digits and `-_.!~*'()` as UTF-8 %XX.
pub fn encode_component(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z'
            | b'a'..=b'z'
            | b'0'..=b'9'
            | b'-'
            | b'_'
            | b'.'
            | b'!'
            | b'~'
            | b'*'
            | b'\''
            | b'('
            | b')' => encoded.push(byte as char),
            _ => encoded.push_str(&format!("%{byte:02X}")),
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_media_id_as_the_browser_does() {
        assert_eq!(encode_component("a b/ü?(1)"), "a%20b%2F%C3%BC%3F(1)");
    }

    #[test]
    fn a_render_plays_as_one_clip_with_its_sound() {
        let timeline: PlayableTimeline = serde_json::from_value(serde_json::json!({
            "id": "t", "name": "AI - Cut", "commit": null, "fps": 25, "width": 1920, "height": 1080,
            "duration": 9, "video": [], "audio": [], "markers": [{ "time": 1, "color": "Blue", "name": "m" }]
        }))
        .unwrap();
        let program = Program::render("/preview/a.mp4", &timeline, 9.5);
        assert!(program.direct);
        assert_eq!(program.timeline.duration, 9.5);
        assert_eq!(program.timeline.video, program.timeline.audio);
        assert_eq!(
            program.timeline.video[0].media.as_deref(),
            Some("/preview/a.mp4")
        );
        assert!(program.timeline.markers.is_empty());
    }
}
