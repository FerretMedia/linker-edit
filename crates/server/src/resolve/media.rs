// Work on media files that needs no Resolve: silences and frames, with the ffmpeg Linker-Edit ships.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use regex::Regex;
use serde::Serialize;

/// Runs ffmpeg; its exit code (1 when it couldn't run or ran out of time) and what it wrote to stderr.
async fn run(ffmpeg: &Path, args: &[String], timeout: Duration) -> (i32, String) {
    let mut command = tokio::process::Command::new(ffmpeg);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    let Ok(child) = command.spawn() else {
        return (1, String::new());
    };
    match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => (
            out.status.code().unwrap_or(1),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        ),
        _ => (1, String::new()),
    }
}

/// Math.round(value * 1000) / 1000.
fn round(value: f64) -> f64 {
    (value * 1000.0 + 0.5).floor() / 1000.0
}

/// The silences in a file, from ffmpeg's silencedetect. Times are seconds from the file's start.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Silences {
    pub file: String,
    #[serde(flatten)]
    pub parsed: ParsedSilences,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ParsedSilences {
    /// Seconds.
    pub duration: f64,
    /// [start, end] in seconds from the file's start, already shrunk by the padding.
    pub silences: Vec<[f64; 2]>,
    /// What's between the silences.
    pub speech: Vec<[f64; 2]>,
    pub silent_seconds: f64,
}

static DURATION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"Duration: (\d+):(\d+):([\d.]+)").unwrap());
static SILENCE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"silence_(start|end): (-?[\d.]+)").unwrap());

/// Number() of a matched number: NaN when it isn't one, like "1.2.3".
fn number(text: &str) -> f64 {
    text.parse().unwrap_or(f64::NAN)
}

/// Reads ffmpeg's silencedetect log. A silence still open at the end runs to the end of the file.
pub fn parse_silences(log: &str, padding: f64) -> ParsedSilences {
    let duration = DURATION.captures(log).map_or(0.0, |time| {
        number(&time[1]) * 3600.0 + number(&time[2]) * 60.0 + number(&time[3])
    });
    let mut raw: Vec<(f64, f64)> = Vec::new();
    let mut open: Option<f64> = None;
    for found in SILENCE.captures_iter(log) {
        // Math.max(0, NaN) is NaN, as f64::max isn't.
        let parsed = number(&found[2]);
        let value = if parsed.is_nan() {
            parsed
        } else {
            parsed.max(0.0)
        };
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
    // Keep a little of each pause next to speech, except at the very start and end of the file.
    let silences: Vec<[f64; 2]> = raw
        .into_iter()
        .map(|(start, end)| {
            let start = if start <= 0.0 { 0.0 } else { start + padding };
            let end = if end >= duration {
                duration
            } else {
                end - padding
            };
            (start, end)
        })
        .filter(|(start, end)| end - start > 0.01)
        .map(|(start, end)| [round(start), round(end)])
        .collect();
    let mut speech = Vec::new();
    let mut at = 0.0;
    for [start, end] in &silences {
        if *start > at {
            speech.push([round(at), round(*start)]);
        }
        at = *end;
    }
    if duration > at {
        speech.push([round(at), round(duration)]);
    }
    let silent: f64 = silences.iter().map(|[start, end]| end - start).sum();
    ParsedSilences {
        duration: round(duration),
        silences,
        speech,
        silent_seconds: round(silent),
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SilenceOptions {
    pub min_silence: f64,
    pub threshold_db: f64,
    pub padding: f64,
}

impl Default for SilenceOptions {
    fn default() -> Self {
        Self {
            min_silence: 0.5,
            threshold_db: -35.0,
            padding: 0.1,
        }
    }
}

static AUDIO: LazyLock<Regex> = LazyLock::new(|| Regex::new("Audio:").unwrap());
static READABLE: LazyLock<Regex> = LazyLock::new(|| Regex::new("silence|Duration").unwrap());

/// A number as JavaScript's String() writes it: 0.5, -35, 1e+21.
pub(crate) fn js_number_text(value: f64) -> String {
    if value.is_nan() {
        "NaN".to_string()
    } else if value.is_infinite() {
        if value > 0.0 { "Infinity" } else { "-Infinity" }.to_string()
    } else if value == 0.0 {
        // -0 too.
        "0".to_string()
    } else {
        // Display writes 10.0 as "10", like JavaScript.
        format!("{value}")
    }
}

pub async fn find_silences(
    ffmpeg: &Path,
    file: &Path,
    options: SilenceOptions,
) -> anyhow::Result<Silences> {
    let filter = format!(
        "silencedetect=noise={}dB:d={}",
        js_number_text(options.threshold_db),
        js_number_text(options.min_silence)
    );
    let args: Vec<String> = ["-hide_banner", "-nostats", "-i"]
        .into_iter()
        .map(String::from)
        .chain([file.to_string_lossy().into_owned()])
        .chain(
            ["-vn", "-af", &filter, "-f", "null", "-"]
                .into_iter()
                .map(String::from),
        )
        .collect();
    let (code, stderr) = run(ffmpeg, &args, Duration::from_secs(10 * 60)).await;
    let file_text = file.to_string_lossy();
    if code != 0 && !READABLE.is_match(&stderr) {
        let last = stderr.trim().split('\n').next_back().unwrap_or("");
        anyhow::bail!("ffmpeg couldn't read {file_text}: {last}");
    }
    if !AUDIO.is_match(&stderr) {
        anyhow::bail!("{file_text} has no audio.");
    }
    Ok(Silences {
        file: file_text.into_owned(),
        parsed: parse_silences(&stderr, options.padding),
    })
}

/// A frame of a media file as a JPEG at most 640 pixels wide.
pub async fn media_frame(ffmpeg: &Path, file: &Path, seconds: f64, out: &Path) -> bool {
    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-ss".into(),
        js_number_text(seconds.max(0.0)),
        "-i".into(),
        file.to_string_lossy().into_owned(),
        "-frames:v".into(),
        "1".into(),
        "-vf".into(),
        "scale='min(640,iw)':-1".into(),
        "-q:v".into(),
        "4".into(),
        out.to_string_lossy().into_owned(),
    ];
    let (code, _) = run(ffmpeg, &args, Duration::from_secs(60)).await;
    code == 0 && out.exists()
}

/// A still Resolve exported, made small for the agent: a JPEG at most 640 pixels wide. Keeps the original when
/// ffmpeg can't.
pub async fn shrink_image(ffmpeg: Option<&Path>, file: &Path) -> PathBuf {
    let Some(ffmpeg) = ffmpeg else {
        return file.to_path_buf();
    };
    let out = file.with_extension("jpg");
    let args: Vec<String> = vec![
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-y".into(),
        "-i".into(),
        file.to_string_lossy().into_owned(),
        "-vf".into(),
        "scale='min(640,iw)':-1".into(),
        "-q:v".into(),
        "4".into(),
        out.to_string_lossy().into_owned(),
    ];
    let (code, _) = run(ffmpeg, &args, Duration::from_secs(60)).await;
    if code != 0 || !out.exists() {
        return file.to_path_buf();
    }
    let _ = std::fs::remove_file(file);
    out
}

/// A fresh folder for frames, under one that drops folders older than an hour.
pub fn frames_dir(root: &Path) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(root)?;
    for entry in std::fs::read_dir(root)?.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let old = metadata
            .modified()
            .ok()
            .and_then(|time| SystemTime::now().duration_since(time).ok())
            .is_some_and(|age| age > Duration::from_secs(60 * 60));
        if metadata.is_dir() && old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
    Ok(tempfile::Builder::new()
        .prefix("frames-")
        .tempdir_in(root)?
        .keep())
}

/// The ffmpeg Linker-Edit ships next to its own program, or one on PATH. The app's packaging puts ffmpeg next to
/// the linker-edit binary; a build from source uses the one on PATH.
pub fn find_ffmpeg() -> anyhow::Result<PathBuf> {
    let name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let bundled = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(name)))
        .filter(|file| file.is_file());
    let on_path = || {
        std::env::var_os("PATH").and_then(|path| {
            std::env::split_paths(&path)
                .map(|dir| dir.join(name))
                .find(|file| file.is_file())
        })
    };
    bundled.or_else(on_path).ok_or_else(|| {
        anyhow::anyhow!(
            "ffmpeg is missing. Reinstall Linker-Edit, or install ffmpeg so it's on your PATH."
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(body: &str) -> String {
        format!("  Duration: 00:00:10.00, start: 0.000000, bitrate: 1 kb/s\n{body}")
    }

    #[test]
    fn pads_silences_next_to_speech_and_lists_the_speech_between_them() {
        let parsed = parse_silences(
            &log(
                "[silencedetect] silence_start: 2\n[silencedetect] silence_end: 4 | silence_duration: 2\n",
            ),
            0.25,
        );
        assert_eq!(
            parsed,
            ParsedSilences {
                duration: 10.0,
                silences: vec![[2.25, 3.75]],
                speech: vec![[0.0, 2.25], [3.75, 10.0]],
                silent_seconds: 1.5
            }
        );
    }

    #[test]
    fn keeps_silences_at_the_very_start_and_end_of_a_file_whole_and_closes_one_still_open() {
        let parsed = parse_silences(
            &log("silence_start: -0.01\nsilence_end: 1 | x\nsilence_start: 8.5\n"),
            0.1,
        );
        assert_eq!(parsed.silences, vec![[0.0, 0.9], [8.6, 10.0]]);
        assert_eq!(parsed.speech, vec![[0.9, 8.6]]);
    }

    #[test]
    fn writes_numbers_for_ffmpeg_like_javascript() {
        assert_eq!(js_number_text(-35.0), "-35");
        assert_eq!(js_number_text(0.5), "0.5");
        assert_eq!(js_number_text(3.0), "3");
    }
}
