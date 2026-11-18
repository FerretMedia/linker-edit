//! Pictures: one ffmpeg per clip, decoding its frames at the stage's size, with the clip's look
//! already applied, as raw BGRA (what GPUI's images hold) on a pipe.
//!
//! Every frame is the whole stage: object-contain's letterbox and whatever a look moves out of
//! the picture are transparent, so the tracks stack as the browser's full-frame elements do. A
//! stream decodes at the timeline's frame rate from where it starts, so frame `n` is due at
//! `first + n / fps`; a still is one frame, for a paused playhead or an image.

use std::io::Read;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};

use gpui::RenderImage;
use smallvec::smallvec;

use super::ffmpeg::{self, Process};
use super::timeline::Transform;
use crate::state::ServerHttp;

/// Frames decoded ahead of the clock. A full pipe holds ffmpeg back.
const BUFFERED: usize = 3;

/// A decoded frame and when it is due on the timeline.
pub struct VideoFrame {
    pub time: f64,
    pub image: Arc<RenderImage>,
}

/// What to decode.
#[derive(Clone, Debug, PartialEq)]
pub struct Decode {
    pub url: String,
    /// Seconds into the source to start at; None for an image.
    pub seek: Option<f64>,
    /// None for one frame; else the frame rate and the clip's speed.
    pub timing: Option<Timing>,
    /// Where the first frame is on the timeline.
    pub first: f64,
    /// Timeline seconds to decode; None for to the end.
    pub length: Option<f64>,
    /// The stage in device pixels.
    pub size: (u32, u32),
    pub transform: Transform,
    pub fit: Fit,
}

/// How the picture fills the frame, as CSS's object-fit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fit {
    /// All of it, with clear bars: the stage.
    Contain,
    /// All of the frame, cropped: the scrubber's thumbnails.
    Cover,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Timing {
    pub fps: f64,
    pub rate: f64,
}

/// The filter graph from the input to `[out]`: fit inside the stage (object-contain), then the
/// look as lookStyle's CSS does it (scale, then rotate, then translate, about the center).
/// Opacity is left to the window, which draws the frame faded.
pub fn filter(
    transform: &Transform,
    (width, height): (u32, u32),
    timing: Option<Timing>,
    fit: Fit,
) -> String {
    // Time starts at the first frame decoded, as the clear background's does: after a seek the
    // picture's first frame can come a hair later, and the overlay would show the background alone.
    let mut chain = String::from("[0:v]");
    match timing {
        Some(Timing { fps, rate }) => {
            chain += &format!("setpts=(PTS-STARTPTS)/{},fps={fps},", rate.abs().max(0.01))
        }
        None => chain += "setpts=PTS-STARTPTS,",
    }
    if fit == Fit::Cover {
        return chain
            + &format!(
                "scale={width}:{height}:force_original_aspect_ratio=increase,\
                 crop={width}:{height},format=bgra[out]"
            );
    }
    chain += &format!("scale={width}:{height}:force_original_aspect_ratio=decrease");
    if !transform.moves_picture() {
        return chain
            + &format!(",format=bgra,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color=black@0[out]");
    }
    let zoom = |scale: f64| scale.abs().clamp(0.01, 8.);
    chain += &format!(
        ",scale=iw*{}:ih*{}",
        zoom(transform.scale_x),
        zoom(transform.scale_y)
    );
    if transform.scale_x < 0. {
        chain += ",hflip";
    }
    if transform.scale_y < 0. {
        chain += ",vflip";
    }
    chain += ",format=bgra";
    if transform.rotate % 360. != 0. {
        // ffmpeg's rotate turns clockwise, as CSS's does; c=none leaves the corners clear.
        let radians = transform.rotate.to_radians();
        chain += &format!(",rotate={radians}:ow=rotw({radians}):oh=roth({radians}):c=none");
    }
    let rate = timing.map_or(String::new(), |timing| format!(":r={}", timing.fps));
    format!(
        "{chain}[fg];color=c=black@0:s={width}x{height}{rate},format=bgra[bg];\
         [bg][fg]overlay=x=(W-w)/2+{x}:y=(H-h)/2+{y}:shortest=1,format=bgra[out]",
        x = transform.x * width as f64,
        y = transform.y * height as f64,
    )
}

/// One clip's frames as ffmpeg decodes them.
pub struct VideoStream {
    pub decode: Decode,
    frames: Receiver<VideoFrame>,
    next: Option<VideoFrame>,
    ended: bool,
    process: Process,
}

impl VideoStream {
    pub fn start(server: &ServerHttp, decode: Decode) -> std::io::Result<VideoStream> {
        let mut args = ffmpeg::input_args(server, &decode.url, decode.seek);
        args.extend([
            "-an".into(),
            "-filter_complex".into(),
            filter(&decode.transform, decode.size, decode.timing, decode.fit),
            "-map".into(),
            "[out]".into(),
        ]);
        match (decode.timing, decode.length) {
            (None, _) => args.extend(["-frames:v".into(), "1".into()]),
            (Some(_), Some(length)) => args.extend(["-t".into(), format!("{:.4}", length.max(0.))]),
            (Some(_), None) => {}
        }
        args.extend(["-f", "rawvideo", "-pix_fmt", "bgra", "pipe:1"].map(String::from));
        let (process, stdout) = ffmpeg::spawn(&args)?;
        let (sender, frames) = sync_channel(BUFFERED);
        let (size, first) = (decode.size, decode.first);
        let step = decode.timing.map_or(0., |timing| 1. / timing.fps);
        std::thread::Builder::new()
            .name("linker-edit-video".into())
            .spawn(move || read_frames(stdout, size, first, step, sender))?;
        Ok(VideoStream {
            decode,
            frames,
            next: None,
            ended: false,
            process,
        })
    }

    pub fn still(&self) -> bool {
        self.decode.timing.is_none()
    }

    fn peek(&mut self) -> Option<&VideoFrame> {
        if self.next.is_none() && !self.ended {
            match self.frames.try_recv() {
                Ok(frame) => self.next = Some(frame),
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.ended = true,
            }
        }
        self.next.as_ref()
    }

    /// When the next decoded frame is due; None until one is decoded.
    pub fn next_time(&mut self) -> Option<f64> {
        self.peek().map(|frame| frame.time)
    }

    /// The latest frame due by `time`, skipping the ones it is too late for; None when the next
    /// is not due yet (or not decoded yet). A still's one frame is always due.
    pub fn take_due(&mut self, time: f64) -> Option<VideoFrame> {
        let still = self.still();
        let mut due = None;
        while self
            .peek()
            .is_some_and(|frame| still || frame.time <= time + 0.001)
        {
            due = self.next.take();
        }
        due
    }

    /// A frame is decoded and waiting, or none will come: playing can start.
    pub fn ready(&mut self) -> bool {
        self.peek().is_some() || self.ended
    }

    /// Ended with nothing more to show.
    pub fn done(&mut self) -> bool {
        self.peek().is_none() && self.ended
    }

    /// None until every frame is taken and ffmpeg has exited; then whether it played the media
    /// (false: it is missing, or not something ffmpeg can play).
    pub fn finished(&mut self) -> Option<bool> {
        if self.done() {
            self.process.exited()
        } else {
            None
        }
    }
}

fn read_frames(
    mut stdout: std::process::ChildStdout,
    (width, height): (u32, u32),
    first: f64,
    step: f64,
    sender: SyncSender<VideoFrame>,
) {
    let length = width as usize * height as usize * 4;
    for n in 0.. {
        let mut bytes = vec![0; length];
        if stdout.read_exact(&mut bytes).is_err() {
            return;
        }
        let Some(buffer) = image::RgbaImage::from_raw(width, height, bytes) else {
            return;
        };
        let image = Arc::new(RenderImage::new(smallvec![image::Frame::new(buffer)]));
        let frame = VideoFrame {
            time: first + n as f64 * step,
            image,
        };
        if sender.send(frame).is_err() {
            return;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn fits_the_picture_in_the_stage_and_applies_the_look() {
        assert_eq!(
            filter(&Transform::IDENTITY, (480, 270), None, Fit::Contain),
            "[0:v]setpts=PTS-STARTPTS,scale=480:270:force_original_aspect_ratio=decrease,\
             format=bgra,pad=480:270:(ow-iw)/2:(oh-ih)/2:color=black@0[out]"
        );
        let look = Transform {
            x: 0.1,
            y: -0.1,
            rotate: -90.,
            scale_x: -1.5,
            scale_y: 1.5,
            opacity: 0.5,
        };
        let radians = (-90f64).to_radians();
        assert_eq!(
            filter(
                &look,
                (480, 270),
                Some(Timing { fps: 24., rate: 2. }),
                Fit::Contain
            ),
            format!(
                "[0:v]setpts=(PTS-STARTPTS)/2,fps=24,\
                 scale=480:270:force_original_aspect_ratio=decrease,scale=iw*1.5:ih*1.5,hflip,format=bgra,\
                 rotate={radians}:ow=rotw({radians}):oh=roth({radians}):c=none[fg];\
                 color=c=black@0:s=480x270:r=24,format=bgra[bg];\
                 [bg][fg]overlay=x=(W-w)/2+48:y=(H-h)/2+-27:shortest=1,format=bgra[out]"
            )
        );
    }

    /// A file of two seconds: red, then blue, with a 440 Hz tone; None without ffmpeg. Made once
    /// per run, and written aside first so a test running beside never reads half of it.
    pub(crate) fn red_then_blue() -> Option<PathBuf> {
        static FILE: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
        FILE.get_or_init(|| {
            let ffmpeg = ffmpeg::path()?;
            let dir = std::env::temp_dir().join("linker-edit-ui-media");
            std::fs::create_dir_all(&dir).unwrap();
            let file = dir.join("red-then-blue.mp4");
            let aside = dir.join(format!("red-then-blue.{}.mp4", std::process::id()));
            let status = std::process::Command::new(ffmpeg)
                .args(["-hide_banner", "-loglevel", "error", "-y"])
                .args(["-f", "lavfi", "-i", "color=c=red:s=64x36:r=30:d=1"])
                .args(["-f", "lavfi", "-i", "color=c=blue:s=64x36:r=30:d=1"])
                .args([
                    "-f",
                    "lavfi",
                    "-i",
                    "sine=frequency=440:sample_rate=48000:duration=2",
                ])
                .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0[v]"])
                .args([
                    "-map", "[v]", "-map", "2:a", "-c:v", "libx264", "-pix_fmt", "yuv420p",
                ])
                .args(["-c:a", "aac"])
                .arg(&aside)
                .status()
                .unwrap();
            assert!(status.success());
            std::fs::rename(&aside, &file).unwrap();
            Some(file)
        })
        .clone()
    }

    pub(crate) fn local() -> ServerHttp {
        ServerHttp {
            base: String::new(),
            bearer: None,
        }
    }

    fn decode(file: &Path, seek: f64, timing: Option<Timing>, size: (u32, u32)) -> Decode {
        Decode {
            url: file.to_string_lossy().into_owned(),
            seek: Some(seek),
            timing,
            first: seek,
            length: Some(0.3),
            size,
            transform: Transform::IDENTITY,
            fit: Fit::Contain,
        }
    }

    /// Every frame until the stream ends; waits on the pipe, not on a clock.
    fn all_frames(mut stream: VideoStream) -> Vec<VideoFrame> {
        let mut frames = Vec::new();
        while let Ok(frame) = stream.frames.recv() {
            frames.push(frame);
        }
        assert_ne!(stream.process.exited(), Some(false));
        frames
    }

    /// The BGRA pixel at (x, y).
    fn pixel(frame: &VideoFrame, x: usize, y: usize) -> [u8; 4] {
        let (width, _) = frame_size(frame);
        let bytes = frame.image.as_bytes(0).unwrap();
        let at = (y * width + x) * 4;
        bytes[at..at + 4].try_into().unwrap()
    }

    fn frame_size(frame: &VideoFrame) -> (usize, usize) {
        let size = frame.image.size(0);
        (size.width.0 as usize, size.height.0 as usize)
    }

    fn red(pixel: [u8; 4]) -> bool {
        let [b, g, r, a] = pixel;
        r > 200 && g < 60 && b < 60 && a == 255
    }

    fn blue(pixel: [u8; 4]) -> bool {
        let [b, g, r, a] = pixel;
        b > 200 && g < 60 && r < 60 && a == 255
    }

    #[test]
    fn decodes_the_frame_at_a_time_and_a_stream_from_it() {
        let Some(file) = red_then_blue() else {
            eprintln!("no ffmpeg: skipped");
            return;
        };
        let server = local();
        // A still lands on the frame: red before the cut, blue after it. The 16:9 picture sits in a
        // square stage with clear bars above and below.
        for (at, is) in [(0.5, red as fn([u8; 4]) -> bool), (1.5, blue)] {
            let still = VideoStream::start(&server, decode(&file, at, None, (64, 64))).unwrap();
            let frames = all_frames(still);
            assert_eq!(frames.len(), 1);
            assert_eq!(frame_size(&frames[0]), (64, 64));
            assert!(
                is(pixel(&frames[0], 32, 32)),
                "at {at}s: {:?}",
                pixel(&frames[0], 32, 32)
            );
            assert_eq!(pixel(&frames[0], 32, 2)[3], 0, "the letterbox is clear");
        }

        // A stream from 0.9s for 0.3s at 30 fps: nine frames a thirtieth apart, the cut at 1s.
        let timing = Some(Timing { fps: 30., rate: 1. });
        let stream = VideoStream::start(&server, decode(&file, 0.9, timing, (32, 18))).unwrap();
        let frames = all_frames(stream);
        assert_eq!(frames.len(), 9);
        assert!((frames[3].time - 1.0).abs() < 1e-9);
        assert!(red(pixel(&frames[0], 16, 9)) && red(pixel(&frames[2], 16, 9)));
        assert!(blue(pixel(&frames[3], 16, 9)) && blue(pixel(&frames[8], 16, 9)));

        // Half the size and moved right by a quarter of the stage: the left half is clear.
        // (Just before a keyframe the picture's first frame comes a hair after the clear
        // background's: the overlay must not take the background alone.)
        let mut moved = decode(&file, 0.95, None, (64, 36));
        moved.transform = Transform {
            x: 0.25,
            scale_x: 0.5,
            scale_y: 0.5,
            ..Transform::IDENTITY
        };
        let frames = all_frames(VideoStream::start(&server, moved).unwrap());
        assert_eq!(pixel(&frames[0], 20, 18)[3], 0);
        assert!(
            red(pixel(&frames[0], 48, 18)),
            "{:?}",
            pixel(&frames[0], 48, 18)
        );

        // A file that isn't there fails, as the browser's onerror.
        let mut missing = decode(&file, 0., None, (8, 8));
        missing.url = file
            .with_file_name("missing.mp4")
            .to_string_lossy()
            .into_owned();
        let mut stream = VideoStream::start(&server, missing).unwrap();
        while stream.frames.recv().is_ok() {}
        stream.process.wait();
        assert_eq!(stream.finished(), Some(false));
    }
}
