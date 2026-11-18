// Transcribing one media file on this machine with whisper.cpp's whisper-cli, and putting its word times back
// where they are in the file.

use std::ffi::{OsStr, OsString};
use std::path::Path;
use std::process::Stdio;
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Context, bail};
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};

use super::tools::{WhisperTools, command};
use crate::js;

/// Times are seconds from the start of the source file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Word {
    #[serde(serialize_with = "js::num")]
    pub start: f64,
    #[serde(serialize_with = "js::num")]
    pub end: f64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Segment {
    #[serde(serialize_with = "js::num")]
    pub start: f64,
    #[serde(serialize_with = "js::num")]
    pub end: f64,
    pub text: String,
    pub words: Vec<Word>,
}

/// A transcript as it is kept in <transcripts dir>/<name>-<hash>.json.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Transcript {
    pub file: String,
    /// Size and modification time of the file when it was transcribed, to spot a changed file.
    pub size: u64,
    #[serde(serialize_with = "js::num")]
    pub modified: f64,
    pub engine: String,
    pub language: String,
    #[serde(serialize_with = "js::num")]
    pub duration: f64,
    pub segments: Vec<Segment>,
}

/// A stretch of speech that voice detection kept, in the original audio and in the audio Whisper heard.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadSegment {
    pub orig_start: f64,
    pub orig_end: f64,
    pub vad_start: f64,
    pub vad_end: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Offsets {
    pub from: f64,
    pub to: f64,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WhisperToken {
    pub text: String,
    pub offsets: Offsets,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WhisperResult {
    pub language: String,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WhisperItem {
    pub offsets: Offsets,
    pub text: String,
    pub tokens: Vec<WhisperToken>,
}

/// The parts of whisper-cli's --output-json-full that we read.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct WhisperJson {
    pub result: WhisperResult,
    pub transcription: Vec<WhisperItem>,
}

/// whisper-cli logs each kept stretch of speech; we need them because it leaves word times unmapped.
pub fn parse_vad_segments(log: &str) -> Vec<VadSegment> {
    static PATTERN: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"vad_segment_info: orig_start: ([0-9.]+), orig_end: ([0-9.]+), vad_start: ([0-9.]+), vad_end: ([0-9.]+)").unwrap()
    });
    PATTERN
        .captures_iter(log)
        .map(|found| VadSegment {
            orig_start: js::parse_number(&found[1]),
            orig_end: js::parse_number(&found[2]),
            vad_start: js::parse_number(&found[3]),
            vad_end: js::parse_number(&found[4]),
        })
        .collect()
}

// whisper.cpp keeps 0.1 s of real audio after each stretch of speech, then inserts 0.1 s of silence.
const VAD_OVERLAP: f64 = 0.1;

/// Which end of a word a time is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    End,
}

/// Maps a time in the speech-only audio Whisper heard back to the original file. In the inserted silence,
/// a word's start belongs to the next stretch of speech and its end to the previous one. whisper.cpp's own
/// mapping spreads those times across the removed silence instead, up to seconds off.
pub fn to_original_time(seconds: f64, vad: &[VadSegment], edge: Edge) -> f64 {
    let mut previous: Option<&VadSegment> = None;
    for segment in vad {
        if seconds < segment.vad_start {
            return match previous {
                Some(previous) if edge == Edge::End => previous.orig_end + VAD_OVERLAP,
                _ => segment.orig_start,
            };
        }
        if seconds <= segment.vad_end + VAD_OVERLAP {
            return segment.orig_start + (seconds - segment.vad_start);
        }
        previous = Some(segment);
    }
    match previous {
        Some(previous) => previous.orig_end + (seconds - previous.vad_end),
        None => seconds,
    }
}

fn round(seconds: f64) -> f64 {
    js::round_to(seconds, 3)
}

/// Whisper's tokens are pieces of words: a new word starts at a token with a leading space. With voice
/// detection on, whisper.cpp leaves token times in the speech-only audio, so they are mapped here, and
/// segments span their words.
pub fn build_segments(json: &WhisperJson, vad: &[VadSegment]) -> Vec<Segment> {
    static SPECIAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?R)^\[_.*\]$").unwrap());
    let mut segments = Vec::new();
    for item in &json.transcription {
        let text = js::trim(&item.text);
        if text.is_empty() {
            continue;
        }
        let mut words: Vec<Word> = Vec::new();
        for token in &item.tokens {
            if token.text.is_empty() || SPECIAL.is_match(&token.text) {
                continue;
            }
            let from = to_original_time(token.offsets.from / 1000.0, vad, Edge::Start);
            let to = from.max(to_original_time(token.offsets.to / 1000.0, vad, Edge::End));
            match words.last_mut() {
                Some(current) if !token.text.starts_with(' ') => {
                    current.text.push_str(&token.text);
                    current.end = to;
                }
                _ => words.push(Word {
                    start: from,
                    end: to,
                    text: js::trim(&token.text).to_string(),
                }),
            }
        }
        let kept: Vec<Word> = words
            .into_iter()
            .filter(|word| !word.text.is_empty())
            .map(|word| Word {
                start: round(word.start),
                end: round(word.end),
                text: word.text,
            })
            .collect();
        segments.push(Segment {
            start: kept
                .first()
                .map_or_else(|| round(item.offsets.from / 1000.0), |word| word.start),
            end: kept
                .last()
                .map_or_else(|| round(item.offsets.to / 1000.0), |word| word.end),
            text: text.to_string(),
            words: kept,
        });
    }
    segments
}

/// Runs a program, with stdout and stderr together as lines, in the order they came.
async fn run(file: &Path, args: &[OsString]) -> std::io::Result<(Option<i32>, String)> {
    let mut child = command(file)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let lines = Arc::new(Mutex::new(Vec::<String>::new()));
    async fn collect(stream: impl AsyncRead + Unpin, lines: Arc<Mutex<Vec<String>>>) {
        let mut reader = BufReader::new(stream);
        let mut buffer = Vec::new();
        while reader
            .read_until(b'\n', &mut buffer)
            .await
            .is_ok_and(|read| read > 0)
        {
            let text = String::from_utf8_lossy(&buffer);
            let text = text.strip_suffix('\n').unwrap_or(&text);
            let text = text.strip_suffix('\r').unwrap_or(text);
            // Like Node's readline, a lone \r ends a line too.
            lines
                .lock()
                .unwrap()
                .extend(text.split('\r').map(str::to_string));
            buffer.clear();
        }
    }
    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    let (_, _, status) = tokio::join!(
        collect(stdout, lines.clone()),
        collect(stderr, lines.clone()),
        child.wait()
    );
    let log = lines.lock().unwrap().join("\n");
    Ok((status?.code(), log))
}

fn tail(log: &str) -> String {
    let lines: Vec<&str> = log.split('\n').filter(|line| !line.is_empty()).collect();
    lines[lines.len().saturating_sub(6)..].join("\n")
}

/// Audio streams and length, from ffmpeg's description of the file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MediaInfo {
    pub audio_streams: usize,
    pub duration: f64,
}

pub fn parse_media_info(log: &str) -> MediaInfo {
    static AUDIO: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?mR)^\s*Stream #[0-9]+:[0-9]+.*: Audio:").unwrap());
    MediaInfo {
        audio_streams: AUDIO.find_iter(log).count(),
        duration: crate::library::scan::parse_duration(log).unwrap_or(0.0),
    }
}

/// Cameras and recorders often put each mic on its own stream, so every audio stream is mixed in.
pub fn extract_args(file: &OsStr, audio_streams: usize, wav: &OsStr) -> Vec<OsString> {
    let mut args: Vec<OsString> = ["-nostdin", "-hide_banner", "-v", "error", "-y", "-i"]
        .map(OsString::from)
        .to_vec();
    args.push(file.to_os_string());
    if audio_streams > 1 {
        let inputs: String = (0..audio_streams).map(|i| format!("[0:a:{i}]")).collect();
        args.push("-filter_complex".into());
        args.push(format!("{inputs}amix=inputs={audio_streams}").into());
    } else {
        args.push("-map".into());
        args.push("0:a:0".into());
    }
    args.extend(["-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le"].map(OsString::from));
    args.push(wav.to_os_string());
    args
}

const WINDOW: f64 = 30.0;

/// Where to sample a clip for its language: Whisper on its own only listens to the first 30 seconds.
pub fn detection_windows(duration: f64, count: usize) -> Vec<f64> {
    if duration <= WINDOW * 1.5 {
        return Vec::new();
    }
    let windows = count.min((duration / WINDOW).ceil() as usize);
    (0..windows)
        .map(|i| round(((duration - WINDOW) * i as f64) / (windows - 1) as f64))
        .collect()
}

/// The language with the most confidence summed over the sampled windows. A clip that opens in English and
/// goes on in Dutch would otherwise be transcribed as English, with the Dutch translated, and badly.
pub fn pick_language(log: &str) -> Option<String> {
    static DETECTED: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"auto-detected language: ([A-Za-z0-9_]+) \(p = ([0-9.]+)\)").unwrap()
    });
    let mut scores: Vec<(String, f64)> = Vec::new();
    for found in DETECTED.captures_iter(log) {
        let score = js::parse_number(&found[2]);
        match scores
            .iter_mut()
            .find(|(language, _)| *language == found[1])
        {
            Some((_, total)) => *total += score,
            None => scores.push((found[1].to_string(), score)),
        }
    }
    scores.sort_by(|a, b| b.1.total_cmp(&a.1));
    scores.into_iter().next().map(|(language, _)| language)
}

async fn detect_language(tools: &WhisperTools, wav: &Path, duration: f64, work: &Path) -> String {
    let windows = detection_windows(duration, 5);
    if windows.is_empty() {
        return "auto".into();
    }
    let mut snippets: Vec<OsString> = Vec::new();
    for (index, start) in windows.iter().enumerate() {
        let snippet = work.join(format!("window-{index}.wav"));
        let args: Vec<OsString> = vec![
            "-nostdin".into(),
            "-v".into(),
            "error".into(),
            "-y".into(),
            "-ss".into(),
            start.to_string().into(),
            "-t".into(),
            WINDOW.to_string().into(),
            "-i".into(),
            wav.into(),
            snippet.clone().into(),
        ];
        if run(&tools.ffmpeg, &args)
            .await
            .is_ok_and(|(code, _)| code == Some(0))
        {
            snippets.push(snippet.into());
        }
    }
    // One whisper-cli run for all windows, so the model loads once.
    let mut args: Vec<OsString> = vec![
        "-m".into(),
        tools.model.clone().into(),
        "-l".into(),
        "auto".into(),
        "-dl".into(),
        "--vad".into(),
        "-vm".into(),
        tools.vad_model.clone().into(),
    ];
    args.extend(snippets);
    match run(&tools.whisper_cli, &args).await {
        Ok((_, log)) => pick_language(&log).unwrap_or_else(|| "auto".into()),
        Err(_) => "auto".into(),
    }
}

/// Transcribes one media file on this machine. `language` is a Whisper code such as "nl", or "auto".
pub async fn transcribe_file(
    tools: &WhisperTools,
    file: &Path,
    language: &str,
) -> anyhow::Result<Transcript> {
    static NO_SPEECH: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"(?i)no speech segments|detected 0 speech segments").unwrap());
    let stat = std::fs::metadata(file)?;
    let (size, modified) = (stat.len(), js::mtime_ms(&stat));
    let header: Vec<OsString> = vec!["-hide_banner".into(), "-i".into(), file.into()];
    let info = parse_media_info(&run(&tools.ffmpeg, &header).await?.1);
    if info.audio_streams == 0 {
        bail!("This file has no audio.");
    }

    let work = tempfile::Builder::new()
        .prefix("linker-edit-transcribe-")
        .tempdir()?;
    let wav = work.path().join("audio.wav");
    let (code, log) = run(
        &tools.ffmpeg,
        &extract_args(file.as_os_str(), info.audio_streams, wav.as_os_str()),
    )
    .await?;
    if code != Some(0) {
        bail!("ffmpeg couldn't read the audio:\n{}", tail(&log));
    }
    let language = if language == "auto" {
        detect_language(tools, &wav, info.duration, work.path()).await
    } else {
        language.to_string()
    };

    let out = work.path().join("transcript");
    let threads = std::thread::available_parallelism()
        .map_or(1, usize::from)
        .min(8);
    let args: Vec<OsString> = vec![
        "-m".into(),
        tools.model.clone().into(),
        "-f".into(),
        wav.into(),
        "-l".into(),
        language.clone().into(),
        "--vad".into(),
        "-vm".into(),
        tools.vad_model.clone().into(),
        "-ojf".into(),
        "-of".into(),
        out.clone().into(),
        "-t".into(),
        threads.to_string().into(),
    ];
    let (code, log) = run(&tools.whisper_cli, &args).await?;
    let json_file = work.path().join("transcript.json");
    let file_text = file.to_string_lossy().into_owned();
    if code != Some(0) || !json_file.exists() {
        // With no speech at all, voice detection leaves Whisper nothing to transcribe.
        if NO_SPEECH.is_match(&log) {
            return Ok(Transcript {
                file: file_text,
                size,
                modified,
                engine: tools.label.clone(),
                language,
                duration: info.duration,
                segments: Vec::new(),
            });
        }
        bail!("Whisper failed:\n{}", tail(&log));
    }
    let bytes = std::fs::read(&json_file)?;
    let json: WhisperJson = serde_json::from_str(&String::from_utf8_lossy(&bytes))
        .context("Whisper's transcript can't be read")?;
    Ok(Transcript {
        file: file_text,
        size,
        modified,
        engine: tools.label.clone(),
        language: json.result.language.clone(),
        duration: info.duration,
        segments: build_segments(&json, &parse_vad_segments(&log)),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real whisper-cli run (large-v3-turbo, voice detection on) over a Dutch screen recording, cut to three
    // segments.
    fn fixture() -> (String, WhisperJson) {
        #[derive(Deserialize)]
        struct Fixture {
            log: String,
            json: WhisperJson,
        }
        let fixture: Fixture =
            serde_json::from_str(include_str!("fixtures/whisper-nl.json")).unwrap();
        (fixture.log, fixture.json)
    }

    #[test]
    fn puts_words_at_their_time_in_the_file_not_in_the_speech_only_audio_whisper_heard() {
        let (log, json) = fixture();
        let segments = build_segments(&json, &parse_vad_segments(&log));
        let starts: Vec<_> = segments
            .iter()
            .map(|segment| (segment.start, segment.text.as_str()))
            .collect();
        assert_eq!(
            starts,
            [
                (1.43, "Aha, dat is dus mijn camera."),
                // Starts in the silence whisper.cpp inserts between stretches of speech: speech resumes at 6.79.
                (6.79, "Oké, anyway, maar check even Tito."),
                (9.77, "Hell ja."),
            ]
        );
        let words = |index: usize| -> Vec<&str> {
            segments[index]
                .words
                .iter()
                .map(|word| word.text.as_str())
                .collect()
        };
        assert_eq!(words(0), ["Aha,", "dat", "is", "dus", "mijn", "camera."]);
        assert_eq!(
            words(1),
            ["Oké,", "anyway,", "maar", "check", "even", "Tito."]
        );
        assert_eq!(
            segments[1].words[3],
            Word {
                start: 7.58,
                end: 7.85,
                text: "check".into()
            }
        );
        for segment in &segments {
            for word in &segment.words {
                assert!(word.start <= word.end);
            }
        }
    }

    #[test]
    fn samples_the_whole_clip_and_picks_the_language_that_wins_overall() {
        assert!(detection_windows(40.0, 5).is_empty());
        assert_eq!(detection_windows(200.0, 5), [0.0, 42.5, 85.0, 127.5, 170.0]);
        // whisper-cli -dl over those windows of a recording that opens in English and goes on in Dutch.
        let log = [
            "whisper_full_with_state: auto-detected language: en (p = 0.624113)",
            "whisper_full_with_state: auto-detected language: nl (p = 0.551040)",
            "whisper_full_with_state: auto-detected language: nl (p = 0.581648)",
            "whisper_full_with_state: auto-detected language: en (p = 0.575867)",
            "whisper_full_with_state: auto-detected language: nl (p = 0.927525)",
        ]
        .join("\n");
        assert_eq!(pick_language(&log).as_deref(), Some("nl"));
        assert_eq!(pick_language("no detections"), None);
    }

    #[test]
    fn mixes_every_audio_stream_into_one_since_recorders_often_put_each_mic_on_its_own() {
        let log = [
            "  Duration: 00:01:02.60, start: 0.000000, bitrate: 6210 kb/s",
            "  Stream #0:0[0x1](und): Video: h264 (High), yuv420p, 1920x1080, 25 fps",
            "  Stream #0:1[0x2](und): Audio: pcm_s24le, 48000 Hz, mono, s32 (24 bit)",
            "  Stream #0:2[0x3](und): Audio: pcm_s24le, 48000 Hz, mono, s32 (24 bit)",
        ]
        .join("\n");
        assert_eq!(
            parse_media_info(&log),
            MediaInfo {
                audio_streams: 2,
                duration: 62.6
            }
        );
        let mixed = extract_args(OsStr::new("in.mxf"), 2, OsStr::new("out.wav"));
        assert!(mixed.contains(&OsString::from("[0:a:0][0:a:1]amix=inputs=2")));
        let single = extract_args(OsStr::new("in.mov"), 1, OsStr::new("out.wav"));
        assert!(single.contains(&OsString::from("0:a:0")));
    }

    #[test]
    fn writes_a_transcript_as_the_typescript_did() {
        let transcript = Transcript {
            file: "a.wav".into(),
            size: 5,
            modified: 1_763_459_200_123.5,
            engine: "test".into(),
            language: "nl".into(),
            duration: 2.0,
            segments: vec![Segment {
                start: 0.5,
                end: 1.0,
                text: "Hallo.".into(),
                words: vec![Word {
                    start: 0.5,
                    end: 1.0,
                    text: "Hallo.".into(),
                }],
            }],
        };
        assert_eq!(
            serde_json::to_string(&transcript).unwrap(),
            r#"{"file":"a.wav","size":5,"modified":1763459200123.5,"engine":"test","language":"nl","duration":2,"segments":[{"start":0.5,"end":1,"text":"Hallo.","words":[{"start":0.5,"end":1,"text":"Hallo."}]}]}"#
        );
    }
}
