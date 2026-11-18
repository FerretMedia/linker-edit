// Measures a video without a model: its size, shots, sound and loudness, and a contact sheet of its frames, with
// the ffmpeg Linker-Edit ships. This is what an agent studies a brand's example videos with.

use std::ffi::OsStr;
use std::path::Path;
use std::sync::LazyLock;
use std::time::Duration;

use anyhow::bail;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::js;
use crate::library::scan::{input_part, parse_duration, parse_loudness};
use crate::transcribe::tools::{RunOutput, run_process};

/// Bump when a Study changes shape or how it is measured, so cached ones are measured again.
pub const STUDY_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shots {
    pub count: usize,
    #[serde(serialize_with = "js::num")]
    pub average_seconds: f64,
    #[serde(serialize_with = "js::num")]
    pub median_seconds: f64,
    #[serde(serialize_with = "js::num")]
    pub longest_seconds: f64,
    #[serde(serialize_with = "js::num")]
    pub cuts_per_minute: f64,
    /// Where the picture changes, in seconds; the first of them when there are very many.
    #[serde(serialize_with = "js::nums")]
    pub cuts: Vec<f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sound {
    /// How much of the video is quiet, 0 to 1. Near 0 usually means music or ambience runs under everything.
    #[serde(serialize_with = "js::num")]
    pub quiet_share: f64,
    /// Quiet stretches of 0.4 seconds or longer.
    pub pauses: usize,
    #[serde(serialize_with = "js::num")]
    pub longest_pause_seconds: f64,
    #[serde(serialize_with = "js::num")]
    pub median_pause_seconds: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContactSheet {
    /// A JPEG of small frames, left to right and top to bottom.
    pub file: String,
    pub frames: u32,
    /// Frame n (from 0) is at about n × this many seconds.
    #[serde(serialize_with = "js::num")]
    pub step_seconds: f64,
}

/// A measured video, as it is kept in <study dir>/<name>-<hash>.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Study {
    pub version: u32,
    pub file: String,
    /// Size and modification time when it was measured, to spot a changed file.
    pub size: u64,
    #[serde(serialize_with = "js::num")]
    pub modified: f64,
    #[serde(serialize_with = "js::num")]
    pub duration: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// "16:9", "9:16", "1:1"…
    pub aspect: Option<String>,
    #[serde(serialize_with = "js::opt_num")]
    pub fps: Option<f64>,
    #[serde(serialize_with = "js::opt_num")]
    pub loudness_lufs: Option<f64>,
    /// None when the picture couldn't be read.
    pub shots: Option<Shots>,
    /// None when the file has no audio.
    pub sound: Option<Sound>,
    pub sheet: Option<ContactSheet>,
}

fn round(value: f64) -> f64 {
    js::round_to(value, 2)
}

async fn run(ffmpeg: &Path, args: &[&OsStr], cancel: Option<&CancellationToken>) -> RunOutput {
    run_process(ffmpeg, args, Duration::from_secs(30 * 60), cancel)
        .await
        .unwrap_or_else(|error| RunOutput {
            code: Some(1),
            log: error.to_string(),
        })
}

const COMMON_ASPECTS: [(f64, &str); 9] = [
    (16.0 / 9.0, "16:9"),
    (9.0 / 16.0, "9:16"),
    (1.0, "1:1"),
    (4.0 / 5.0, "4:5"),
    (4.0 / 3.0, "4:3"),
    (3.0 / 4.0, "3:4"),
    (21.0 / 9.0, "21:9"),
    (2.39, "2.39:1"),
    (1.9, "1.9:1"),
];

pub fn aspect_name(width: u32, height: u32) -> String {
    let ratio = f64::from(width) / f64::from(height);
    match COMMON_ASPECTS
        .iter()
        .find(|(value, _)| (value - ratio).abs() / value < 0.02)
    {
        Some((_, name)) => name.to_string(),
        None => format!("{}:1", round(ratio)),
    }
}

/// What parse_video_info reads from ffmpeg's log.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub duration: f64,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub aspect: Option<String>,
    pub fps: Option<f64>,
    pub audio: bool,
}

/// The picture's size as it is shown (phones record sideways and say so), frame rate and length, from ffmpeg's log.
pub fn parse_video_info(log: &str) -> VideoInfo {
    static VIDEO: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r"Stream #[0-9]+:[0-9]+[^\n]*: Video: [^\n]*?\b([0-9]{2,5})x([0-9]{2,5})\b([^\n]*)",
        )
        .unwrap()
    });
    static ROTATION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"rotation of (-?[0-9.]+) degrees").unwrap());
    static RATE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"([0-9.]+) fps").unwrap());
    static AUDIO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"Stream #[0-9]+:[0-9]+[^\n]*: Audio:").unwrap());
    let input = input_part(log);
    let duration = parse_duration(input).unwrap_or(0.0);
    let video = VIDEO.captures(input);
    let size = |index: usize| {
        video
            .as_ref()
            .and_then(|video| video[index].parse::<u32>().ok())
    };
    let (mut width, mut height) = (size(1), size(2));
    if let (Some(w), Some(h), Some(rotation)) = (width, height, ROTATION.captures(input)) {
        let quarter_turns = js::math_round(js::parse_number(&rotation[1]) / 90.0).abs();
        if w > 0 && h > 0 && quarter_turns % 2.0 == 1.0 {
            (width, height) = (Some(h), Some(w));
        }
    }
    let fps = video.as_ref().and_then(|video| {
        RATE.captures(&video[3])
            .map(|rate| js::parse_number(&rate[1]))
    });
    let aspect = match (width, height) {
        (Some(w), Some(h)) if w > 0 && h > 0 => Some(aspect_name(w, h)),
        _ => None,
    };
    VideoInfo {
        duration: js::round_to(duration, 3),
        width,
        height,
        aspect,
        fps,
        audio: AUDIO.is_match(input),
    }
}

const MAX_CUTS: usize = 400;

/// Shot lengths from the times the picture changes (ffmpeg's showinfo after a scene filter).
pub fn parse_shots(log: &str, duration: f64) -> Option<Shots> {
    static TIME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"pts_time:([0-9.]+)").unwrap());
    if duration <= 0.0 {
        return None;
    }
    let cuts: Vec<f64> = TIME
        .captures_iter(log)
        .map(|found| js::parse_number(&found[1]))
        .filter(|time| *time > 0.0 && *time < duration)
        .collect();
    let mut edges = vec![0.0];
    edges.extend(&cuts);
    edges.push(duration);
    let mut lengths: Vec<f64> = edges.windows(2).map(|pair| pair[1] - pair[0]).collect();
    lengths.sort_by(f64::total_cmp);
    Some(Shots {
        count: lengths.len(),
        average_seconds: round(duration / lengths.len() as f64),
        median_seconds: round(lengths[lengths.len() / 2]),
        longest_seconds: round(lengths[lengths.len() - 1]),
        cuts_per_minute: js::round_to((cuts.len() as f64 / duration) * 60.0, 1),
        cuts: cuts
            .iter()
            .take(MAX_CUTS)
            .map(|time| round(*time))
            .collect(),
    })
}

/// Quiet stretches from ffmpeg's silencedetect log, and their total. The same as resolve/media.ts's parseSilences
/// with no padding, which is all study needs; it rounds to milliseconds.
fn parse_silences(log: &str) -> (f64, Vec<(f64, f64)>, f64) {
    let round = |value| js::round_to(value, 3);
    static SILENCE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"silence_(start|end): (-?[0-9.]+)").unwrap());
    let duration = parse_duration(log).unwrap_or(0.0);
    let mut raw = Vec::new();
    let mut open: Option<f64> = None;
    for found in SILENCE.captures_iter(log) {
        let value = js::parse_number(&found[2]).max(0.0);
        if &found[1] == "start" {
            open = Some(value);
        } else if let Some(start) = open.take() {
            raw.push((start, value));
        }
    }
    if let Some(start) = open
        && duration > start
    {
        raw.push((start, duration));
    }
    let silences: Vec<(f64, f64)> = raw
        .into_iter()
        .map(|(start, end)| {
            (
                if start <= 0.0 { 0.0 } else { start },
                if end >= duration { duration } else { end },
            )
        })
        .filter(|(start, end)| end - start > 0.01)
        .map(|(start, end)| (round(start), round(end)))
        .collect();
    let silent = silences.iter().map(|(start, end)| end - start).sum::<f64>();
    (round(duration), silences, round(silent))
}

/// How quiet the video gets, from ffmpeg's silencedetect log.
pub fn parse_sound(log: &str) -> Sound {
    let (duration, silences, silent_seconds) = parse_silences(log);
    let mut lengths: Vec<f64> = silences.iter().map(|(start, end)| end - start).collect();
    lengths.sort_by(f64::total_cmp);
    Sound {
        quiet_share: if duration > 0.0 {
            round(silent_seconds / duration)
        } else {
            0.0
        },
        pauses: lengths.len(),
        longest_pause_seconds: round(lengths.last().copied().unwrap_or(0.0)),
        median_pause_seconds: round(lengths.get(lengths.len() / 2).copied().unwrap_or(0.0)),
    }
}

/// The contact sheet's grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SheetLayout {
    pub columns: u32,
    pub rows: u32,
    pub cell_width: u32,
    pub cell_height: u32,
}

/// The contact sheet's grid: wide videos get 6 across, tall ones 8, so both end up about as big.
pub fn sheet_layout(width: Option<u32>, height: Option<u32>) -> SheetLayout {
    let (width, height) = (width.unwrap_or(0), height.unwrap_or(0));
    let tall = width > 0 && height > 0 && height > width;
    let cell_width = if tall { 180 } else { 320 };
    let ratio = if width > 0 && height > 0 {
        f64::from(height) / f64::from(width)
    } else {
        9.0 / 16.0
    };
    // Even, as JPEG encoders want.
    let cell_height = (js::math_round((f64::from(cell_width) * ratio) / 2.0) * 2.0).max(2.0) as u32;
    let (columns, rows) = if tall { (8, 3) } else { (6, 4) };
    SheetLayout {
        columns,
        rows,
        cell_width,
        cell_height,
    }
}

/// Measures one video. sheet_file is where its contact sheet goes.
pub async fn measure_video(
    ffmpeg: &Path,
    file: &Path,
    sheet_file: &Path,
    cancel: Option<&CancellationToken>,
) -> anyhow::Result<Study> {
    let stat = std::fs::metadata(file)?;
    let input = file.as_os_str();
    fn os(args: &[&'static str]) -> Vec<&'static OsStr> {
        args.iter().map(|arg| OsStr::new(*arg)).collect()
    }
    let mut args = os(&["-hide_banner", "-i"]);
    args.push(input);
    let header = run(ffmpeg, &args, cancel).await;
    let info = parse_video_info(&header.log);
    if info.duration <= 0.0 {
        let last = js::trim(&header.log).rsplit('\n').next().unwrap_or("");
        bail!("ffmpeg couldn't read {}: {last}", file.display());
    }

    let mut sound = None;
    let mut loudness = None;
    if info.audio {
        let mut args = os(&["-hide_banner", "-nostats", "-i"]);
        args.push(input);
        args.extend(os(&[
            "-map",
            "0:a:0",
            "-af",
            "silencedetect=noise=-35dB:d=0.4,ebur128=framelog=verbose",
            "-f",
            "null",
            "-",
        ]));
        let audio = run(ffmpeg, &args, cancel).await;
        sound = Some(parse_sound(&audio.log));
        loudness = parse_loudness(&audio.log);
    }

    let mut shots = None;
    let mut sheet = None;
    if let (Some(width), Some(height)) = (info.width, info.height)
        && width > 0
        && height > 0
    {
        // Made small first: finding cuts doesn't need the full picture, and it is many times faster.
        let mut args = os(&["-hide_banner", "-nostats", "-i"]);
        args.push(input);
        args.extend(os(&[
            "-an",
            "-vf",
            "scale=320:-2,select='gt(scene,0.3)',showinfo",
            "-f",
            "null",
            "-",
        ]));
        let cuts = run(ffmpeg, &args, cancel).await;
        shots = if cuts.code == Some(0) {
            parse_shots(&cuts.log, info.duration)
        } else {
            None
        };

        let layout = sheet_layout(info.width, info.height);
        let frames = layout.columns * layout.rows;
        let filter = format!(
            "fps={frames}/{},scale={}:{},tile={}x{}",
            info.duration, layout.cell_width, layout.cell_height, layout.columns, layout.rows
        );
        let mut args = os(&["-hide_banner", "-loglevel", "error", "-y", "-i"]);
        args.push(input);
        args.extend(os(&["-an", "-vf"]));
        args.push(OsStr::new(&filter));
        args.extend(os(&["-frames:v", "1", "-q:v", "4"]));
        args.push(sheet_file.as_os_str());
        let made = run(ffmpeg, &args, cancel).await;
        if made.code == Some(0) && sheet_file.exists() {
            sheet = Some(ContactSheet {
                file: sheet_file.to_string_lossy().into_owned(),
                frames,
                step_seconds: round(info.duration / f64::from(frames)),
            });
        }
    }

    if cancel.is_some_and(CancellationToken::is_cancelled) {
        bail!("Stopped.");
    }
    Ok(Study {
        version: STUDY_VERSION,
        file: file.to_string_lossy().into_owned(),
        size: stat.len(),
        modified: js::mtime_ms(&stat),
        duration: info.duration,
        width: info.width,
        height: info.height,
        aspect: info.aspect,
        fps: info.fps,
        loudness_lufs: loudness,
        shots,
        sound,
        sheet,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::testing::{temp, test_ffmpeg};

    #[test]
    fn reads_the_picture_as_it_is_shown_the_frame_rate_and_whether_there_is_sound() {
        let log = "Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'reel.mp4':
  Duration: 00:00:34.50, start: 0.000000, bitrate: 8000 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 7800 kb/s, 29.97 fps, 29.97 tbr, 30k tbn (default)
    Side data:
      displaymatrix: rotation of -90.00 degrees
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 192 kb/s (default)";
        // A phone records sideways and says so: the video is tall.
        assert_eq!(
            parse_video_info(log),
            VideoInfo {
                duration: 34.5,
                width: Some(1080),
                height: Some(1920),
                aspect: Some("9:16".into()),
                fps: Some(29.97),
                audio: true
            }
        );
        let side = Regex::new(r"\n {4}Side data:\n.*degrees").unwrap();
        let audio = Regex::new(r"\n.*Audio:.*").unwrap();
        let flat = audio.replace(&side.replace(log, ""), "").into_owned();
        let info = parse_video_info(&flat);
        assert_eq!(
            (info.width, info.height, info.aspect.as_deref(), info.audio),
            (Some(1920), Some(1080), Some("16:9"), false)
        );
        assert_eq!(
            parse_video_info("reel.mp4: Invalid data found when processing input").duration,
            0.0
        );
    }

    #[test]
    fn turns_the_times_the_picture_changes_into_shot_lengths() {
        let log = "[Parsed_showinfo_2] n:0 pts:60 pts_time:2 \n[Parsed_showinfo_2] n:1 pts:150 pts_time:5\n[Parsed_showinfo_2] n:2 pts:300 pts_time:10";
        assert_eq!(
            parse_shots(log, 20.0),
            Some(Shots {
                count: 4,
                average_seconds: 5.0,
                median_seconds: 5.0,
                longest_seconds: 10.0,
                cuts_per_minute: 9.0,
                cuts: vec![2.0, 5.0, 10.0]
            })
        );
        // One shot, start to end.
        let one = parse_shots("", 12.0).unwrap();
        assert_eq!(
            (one.count, one.average_seconds, one.cuts.len()),
            (1, 12.0, 0)
        );
        assert_eq!(parse_shots("", 0.0), None);
    }

    #[test]
    fn says_how_quiet_a_video_gets() {
        let log = "  Duration: 00:00:20.00, start: 0.000000
[silencedetect @ 1] silence_start: 4
[silencedetect @ 1] silence_end: 5 | silence_duration: 1
[silencedetect @ 1] silence_start: 10
[silencedetect @ 1] silence_end: 13 | silence_duration: 3";
        assert_eq!(
            parse_sound(log),
            Sound {
                quiet_share: 0.2,
                pauses: 2,
                longest_pause_seconds: 3.0,
                median_pause_seconds: 3.0
            }
        );
        // Music under everything: never quiet.
        assert_eq!(
            parse_sound("  Duration: 00:00:20.00"),
            Sound {
                quiet_share: 0.0,
                pauses: 0,
                longest_pause_seconds: 0.0,
                median_pause_seconds: 0.0
            }
        );
    }

    #[test]
    fn lays_a_tall_videos_contact_sheet_out_in_more_narrower_frames() {
        assert_eq!(
            sheet_layout(Some(1920), Some(1080)),
            SheetLayout {
                columns: 6,
                rows: 4,
                cell_width: 320,
                cell_height: 180
            }
        );
        assert_eq!(
            sheet_layout(Some(1080), Some(1920)),
            SheetLayout {
                columns: 8,
                rows: 3,
                cell_width: 180,
                cell_height: 320
            }
        );
    }

    #[tokio::test]
    async fn measures_a_real_video_with_ffmpeg() {
        let Some(ffmpeg) = test_ffmpeg() else {
            return;
        };
        let dir = temp();
        let video = dir.path().join("clip.mp4");
        let args: Vec<&OsStr> = [
            "-hide_banner",
            "-loglevel",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x180:rate=25:duration=3",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:duration=3",
            "-shortest",
            "-pix_fmt",
            "yuv420p",
        ]
        .iter()
        .map(OsStr::new)
        .chain([video.as_os_str()])
        .collect();
        let made = run_process(&ffmpeg, &args, Duration::from_secs(60), None)
            .await
            .unwrap();
        assert_eq!(made.code, Some(0), "{}", made.log);
        let sheet = dir.path().join("clip.jpg");
        let study = measure_video(&ffmpeg, &video, &sheet, None).await.unwrap();
        assert_eq!(
            (study.width, study.height, study.aspect.as_deref()),
            (Some(320), Some(180), Some("16:9"))
        );
        assert_eq!(study.fps, Some(25.0));
        assert!((study.duration - 3.0).abs() < 0.1, "{}", study.duration);
        assert!(study.loudness_lufs.is_some());
        assert!(study.sound.is_some() && study.shots.is_some());
        assert_eq!(study.sheet.as_ref().map(|sheet| sheet.frames), Some(24));
        assert!(sheet.exists());
    }
}
