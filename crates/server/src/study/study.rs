// The agents' study tools on the "linker" MCP server: measuring finished videos (study_media) and lining a
// finished video up with its raw footage (compare_cut). Neither needs Resolve.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

use anyhow::{anyhow, bail};
use async_trait::async_trait;
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::compare::{CutComparison, compare_cut, tokens};
use super::measure::{STUDY_VERSION, Shots, Sound, Study, measure_video};
use crate::js;
use crate::library::scan::{FindFfmpeg, VIDEO};
use crate::transcribe::transcriber::{Transcriber, TranscriptResult};
use crate::transcribe::whisper::Transcript;

pub const STUDY_MEDIA: &str = "study_media";
pub const COMPARE_CUT: &str = "compare_cut";

/// The tools' definitions, as the MCP server lists them.
pub fn study_tools() -> Vec<Value> {
    vec![
        json!({
            "name": STUDY_MEDIA,
            "description": "Measure finished videos, to learn how they are edited: length, aspect ratio, frame rate, loudness, shot lengths and cuts per minute, how quiet it gets, words per minute, and how it opens and closes. Shows you a contact sheet of each: small frames across the whole video, left to right and top to bottom. Pass video files or folders of them, at most 8 videos per call. Results are kept until a file changes. It misses cuts between similar frames (jump cuts in a talking head), and it can't tell music from other sound.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "files": { "type": "array", "items": { "type": "string" }, "description": "Absolute paths of video files, or of folders with videos in them." },
                    "transcribe": { "type": "boolean", "description": "Also transcribe each video, for words per minute and its opening and closing words. Defaults to true." },
                },
                "required": ["files"],
            },
            "annotations": { "readOnlyHint": true },
        }),
        json!({
            "name": COMPARE_CUT,
            "description": "Line a finished video up with the raw footage it was cut from, by what is said in both. Shows what the editor kept (with times in both), what they cut and whether that was a retake or content left out, which take they used when something was said more than once, whether they reordered, and how much silence they left at cuts. Needs speech in both; transcribes them first.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "final": { "type": "string", "description": "Absolute path of the finished video." },
                    "raw": { "type": "array", "items": { "type": "string" }, "description": "Absolute paths of the raw footage files, or of folders with them." },
                    "language": { "type": "string", "description": "Spoken language as a Whisper code such as \"en\" or \"nl\". Leave out to detect it per file." },
                },
                "required": ["final", "raw"],
            },
            "annotations": { "readOnlyHint": true },
        }),
    ]
}

pub fn is_study_tool(name: &str) -> bool {
    name == STUDY_MEDIA || name == COMPARE_CUT
}

const MAX_STUDIED: usize = 8;
const MAX_RAW: usize = 60;
const OPENING_WORDS: usize = 40;
const CLOSING_WORDS: usize = 25;

/// Transcribes files for the study tools: the Transcriber, or a stand-in in tests.
#[async_trait]
pub trait Transcribe: Send + Sync {
    async fn transcribe(
        &self,
        files: Vec<String>,
        language: Option<String>,
    ) -> Vec<TranscriptResult>;
}

#[async_trait]
impl Transcribe for Transcriber {
    async fn transcribe(
        &self,
        files: Vec<String>,
        language: Option<String>,
    ) -> Vec<TranscriptResult> {
        Transcriber::transcribe(self, &files, language.as_deref()).await
    }
}

/// Measures one video: measure::measure_video, or a stand-in in tests.
#[async_trait]
pub trait MeasureVideo: Send + Sync {
    async fn measure(
        &self,
        ffmpeg: &Path,
        file: &Path,
        sheet_file: &Path,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<Study>;
}

struct Ffmpeg;

#[async_trait]
impl MeasureVideo for Ffmpeg {
    async fn measure(
        &self,
        ffmpeg: &Path,
        file: &Path,
        sheet_file: &Path,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<Study> {
        measure_video(ffmpeg, file, sheet_file, cancel).await
    }
}

pub struct StudyOptions {
    /// Where measurements and contact sheets are kept: the TypeScript used <library cache dir>/../study.
    pub dir: PathBuf,
    pub ffmpeg: FindFfmpeg,
    pub transcribe: Arc<dyn Transcribe>,
    /// measure_video when None.
    pub measure: Option<Arc<dyn MeasureVideo>>,
}

fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter(|item| !js::trim(item).is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn round(value: f64) -> f64 {
    js::round_to(value, 2)
}

fn has_video_ext(name: &str) -> bool {
    VIDEO.contains(&js::extname(name).to_lowercase().as_str())
}

/// The video files a list of files and folders stands for, folders read one level deep and in name order.
pub fn video_files(paths: &[String]) -> anyhow::Result<Vec<String>> {
    let mut files: Vec<String> = Vec::new();
    for entry in paths {
        let full = js::resolve(entry);
        let Ok(stat) = std::fs::metadata(&full) else {
            bail!("There is no file or folder {entry} on this computer.");
        };
        if stat.is_file() {
            files.push(full.to_string_lossy().into_owned());
            continue;
        }
        let mut inside: Vec<String> = std::fs::read_dir(&full)?
            .filter_map(Result::ok)
            .filter(|item| {
                let name = item.file_name().to_string_lossy().into_owned();
                item.file_type().is_ok_and(|kind| kind.is_file())
                    && !name.starts_with('.')
                    && has_video_ext(&name)
            })
            .map(|item| full.join(item.file_name()).to_string_lossy().into_owned())
            .collect();
        inside.sort_by(|a, b| js::locale_compare(a, b));
        if inside.is_empty() {
            bail!(
                "{entry} has no video files in it. Folders inside it aren't read: pass them too."
            );
        }
        files.extend(inside);
    }
    let mut seen = std::collections::HashSet::new();
    files.retain(|file| seen.insert(file.clone()));
    Ok(files)
}

/// Words per minute of talk, and the first and last words, from a transcript.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Speech {
    pub language: String,
    pub words: usize,
    #[serde(serialize_with = "js::num")]
    pub words_per_minute: f64,
    /// How much of the video someone is talking, 0 to 1.
    #[serde(serialize_with = "js::num")]
    pub speech_share: f64,
    #[serde(serialize_with = "js::opt_num")]
    pub first_word_at_seconds: Option<f64>,
    pub opens_with: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closes_with: Option<String>,
}

pub fn speech_of(transcript: &Transcript) -> Speech {
    let words = tokens(transcript);
    let talk: f64 = transcript
        .segments
        .iter()
        .map(|segment| (segment.end - segment.start).max(0.0))
        .sum();
    let all: Vec<&str> = transcript
        .segments
        .iter()
        .flat_map(|segment| segment.words.iter().map(|word| word.text.as_str()))
        .collect();
    Speech {
        language: transcript.language.clone(),
        words: words.len(),
        words_per_minute: if talk > 0.0 {
            js::math_round((words.len() as f64 / talk) * 60.0)
        } else {
            0.0
        },
        speech_share: if transcript.duration > 0.0 {
            round((talk / transcript.duration).min(1.0))
        } else {
            0.0
        },
        first_word_at_seconds: words.first().map(|word| round(word.start)),
        opens_with: all
            .iter()
            .take(OPENING_WORDS)
            .copied()
            .collect::<Vec<_>>()
            .join(" "),
        closes_with: (all.len() > OPENING_WORDS + CLOSING_WORDS)
            .then(|| all[all.len() - CLOSING_WORDS..].join(" ")),
    }
}

/// What a study tool gives the agent: its text, and images to show (contact sheets, by path).
#[derive(Debug, Clone, PartialEq)]
pub struct StudyOutput {
    pub text: String,
    pub images: Vec<String>,
}

#[derive(Serialize)]
struct SheetRef {
    image: usize,
    frames: u32,
    #[serde(serialize_with = "js::num")]
    step_seconds: f64,
}

#[derive(Serialize)]
struct StudiedVideo {
    file: String,
    #[serde(serialize_with = "js::num")]
    duration: f64,
    width: Option<u32>,
    height: Option<u32>,
    aspect: Option<String>,
    #[serde(serialize_with = "js::opt_num")]
    fps: Option<f64>,
    #[serde(serialize_with = "js::opt_num")]
    loudness_lufs: Option<f64>,
    shots: Option<Shots>,
    sound: Option<Sound>,
    // Some(None) is "speech": null, when the transcript failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    speech: Option<Option<Speech>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcript: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    transcript_problem: Option<String>,
    contact_sheet: Option<SheetRef>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum VideoEntry {
    Studied(Box<StudiedVideo>),
    Failed { file: String, error: String },
}

#[derive(Serialize)]
struct StudyAnswer {
    videos: Vec<VideoEntry>,
    note: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    not_studied_yet: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    more: Option<String>,
}

#[derive(Serialize)]
struct CompareAnswer {
    #[serde(flatten)]
    comparison: CutComparison,
    #[serde(skip_serializing_if = "Option::is_none")]
    raw_not_transcribed: Option<Vec<String>>,
}

struct Inner {
    options: StudyOptions,
    // One at a time: each measurement decodes a whole video.
    queue: tokio::sync::Mutex<()>,
}

/// The study tools' service. Cheap to clone.
#[derive(Clone)]
pub struct StudyService {
    inner: Arc<Inner>,
}

impl StudyService {
    pub fn new(options: StudyOptions) -> Self {
        Self {
            inner: Arc::new(Inner {
                options,
                queue: tokio::sync::Mutex::new(()),
            }),
        }
    }

    fn cache_files(&self, file: &str) -> (PathBuf, PathBuf) {
        static UNSAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9_.-]+").unwrap());
        let key_source = if cfg!(windows) {
            file.to_lowercase()
        } else {
            file.to_string()
        };
        let key = &js::sha256_hex(&key_source)[..12];
        let name = js::parsed_name(Path::new(file));
        let name = UNSAFE.replace_all(&name, "_");
        let base = self
            .inner
            .options
            .dir
            .join(format!("{}-{key}", js::slice(&name, 60)));
        let base = base.to_string_lossy();
        (
            PathBuf::from(format!("{base}.json")),
            PathBuf::from(format!("{base}.jpg")),
        )
    }

    fn cached(&self, file: &str) -> Option<Study> {
        let (json, _) = self.cache_files(file);
        let study: Study = serde_json::from_str(&std::fs::read_to_string(json).ok()?).ok()?;
        let stat = std::fs::metadata(file).ok()?;
        if study.version != STUDY_VERSION
            || study.size != stat.len()
            || study.modified != js::mtime_ms(&stat)
        {
            return None;
        }
        if let Some(sheet) = &study.sheet
            && !Path::new(&sheet.file).exists()
        {
            return None;
        }
        Some(study)
    }

    async fn measure(
        &self,
        file: &str,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<Study> {
        if let Some(hit) = self.cached(file) {
            return Ok(hit);
        }
        let _turn = self.inner.queue.lock().await;
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            bail!("Stopped.");
        }
        let options = &self.inner.options;
        std::fs::create_dir_all(&options.dir)?;
        let (json, sheet) = self.cache_files(file);
        let ffmpeg = (options.ffmpeg)()?;
        let measure: Arc<dyn MeasureVideo> =
            options.measure.clone().unwrap_or_else(|| Arc::new(Ffmpeg));
        let study = measure
            .measure(&ffmpeg, Path::new(file), &sheet, cancel)
            .await?;
        std::fs::write(json, serde_json::to_string(&study)?)?;
        Ok(study)
    }

    async fn study_media(
        &self,
        input: &Map<String, Value>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<StudyOutput> {
        let files = video_files(&strings(input.get("files")))?;
        if files.is_empty() {
            bail!("Pass video files or folders in files.");
        }
        let now: Vec<String> = files.iter().take(MAX_STUDIED).cloned().collect();
        let transcripts = if input.get("transcribe") == Some(&Value::Bool(false)) {
            Vec::new()
        } else {
            self.inner
                .options
                .transcribe
                .transcribe(now.clone(), None)
                .await
        };
        let mut images: Vec<String> = Vec::new();
        let mut videos = Vec::new();
        for (index, file) in now.iter().enumerate() {
            match self.measure(file, cancel).await {
                Ok(study) => {
                    if let Some(sheet) = &study.sheet {
                        images.push(sheet.file.clone());
                    }
                    let (speech, transcript, transcript_problem) = match transcripts.get(index) {
                        Some(TranscriptResult::Done {
                            transcript,
                            text_file,
                            ..
                        }) => (
                            Some(Some(speech_of(transcript))),
                            Some(text_file.clone()),
                            None,
                        ),
                        Some(TranscriptResult::Failed { error, .. }) => {
                            (Some(None), None, Some(error.clone()))
                        }
                        None => (None, None, None),
                    };
                    videos.push(VideoEntry::Studied(Box::new(StudiedVideo {
                        file: study.file,
                        duration: study.duration,
                        width: study.width,
                        height: study.height,
                        aspect: study.aspect,
                        fps: study.fps,
                        loudness_lufs: study.loudness_lufs,
                        shots: study.shots,
                        sound: study.sound,
                        speech,
                        transcript,
                        transcript_problem,
                        contact_sheet: study.sheet.map(|sheet| SheetRef {
                            image: images.len(),
                            frames: sheet.frames,
                            step_seconds: sheet.step_seconds,
                        }),
                    })));
                }
                Err(error) => {
                    if cancel.is_some_and(CancellationToken::is_cancelled) {
                        return Err(error);
                    }
                    videos.push(VideoEntry::Failed {
                        file: file.clone(),
                        error: error.to_string(),
                    });
                }
            }
        }
        let left: Vec<String> = files.iter().skip(MAX_STUDIED).cloned().collect();
        let more = (!left.is_empty())
            .then(|| format!("{} more videos: call again with these.", left.len()));
        let answer = StudyAnswer {
            videos,
            note: "contact_sheet.image is which of the attached images shows that video, counted from 1. Frame n of a sheet (from 0) is at about n × step_seconds.",
            not_studied_yet: (!left.is_empty()).then_some(left),
            more,
        };
        Ok(StudyOutput {
            text: serde_json::to_string(&answer)?,
            images,
        })
    }

    async fn compare(&self, input: &Map<String, Value>) -> anyhow::Result<StudyOutput> {
        let r#final = match input.get("final").and_then(Value::as_str) {
            Some(file) => video_files(&[file.to_string()])?,
            None => Vec::new(),
        };
        if r#final.len() != 1 {
            bail!("Pass the one finished video in final.");
        }
        let final_file = r#final[0].clone();
        let raw: Vec<String> = video_files(&strings(input.get("raw")))?
            .into_iter()
            .filter(|file| *file != final_file)
            .collect();
        if raw.is_empty() {
            bail!("Pass the raw footage files or folders in raw.");
        }
        if raw.len() > MAX_RAW {
            bail!(
                "That is {} raw files; pass at most {MAX_RAW}, the ones this video was cut from.",
                raw.len()
            );
        }
        let language = input
            .get("language")
            .and_then(Value::as_str)
            .map(str::to_string);
        let mut all = vec![final_file.clone()];
        all.extend(raw);
        let mut results = self
            .inner
            .options
            .transcribe
            .transcribe(all, language)
            .await
            .into_iter();
        let final_transcript = match results.next() {
            Some(TranscriptResult::Done { transcript, .. }) => transcript,
            Some(TranscriptResult::Failed { error, .. }) => {
                bail!("Couldn't transcribe {final_file}: {error}")
            }
            None => bail!("Couldn't transcribe {final_file}: no result"),
        };
        let mut usable = Vec::new();
        let mut failed = Vec::new();
        for result in results {
            match result {
                TranscriptResult::Done { transcript, .. } => usable.push(transcript),
                TranscriptResult::Failed { file, .. } => failed.push(file),
            }
        }
        if usable.is_empty() {
            bail!("None of the raw files could be transcribed.");
        }
        if tokens(&final_transcript).is_empty() {
            return Err(anyhow!(
                "Nobody speaks in the finished video, so there is nothing to line up."
            ));
        }
        let answer = CompareAnswer {
            comparison: compare_cut(&final_transcript, &usable),
            raw_not_transcribed: (!failed.is_empty()).then_some(failed),
        };
        Ok(StudyOutput {
            text: serde_json::to_string(&answer)?,
            images: Vec::new(),
        })
    }

    /// Runs one of the study tools; cancel stops a measurement when the turn is stopped.
    pub async fn run(
        &self,
        tool: &str,
        input: &Map<String, Value>,
        cancel: Option<&CancellationToken>,
    ) -> anyhow::Result<StudyOutput> {
        if tool == STUDY_MEDIA {
            self.study_media(input, cancel).await
        } else {
            self.compare(input).await
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::library::testing::temp;
    use crate::study::compare::tests::transcript;
    use crate::study::measure::ContactSheet;

    struct FakeTranscribe;

    #[async_trait]
    impl Transcribe for FakeTranscribe {
        async fn transcribe(&self, files: Vec<String>, _: Option<String>) -> Vec<TranscriptResult> {
            files
                .into_iter()
                .map(|file| {
                    if file.ends_with("b.mp4") {
                        TranscriptResult::Failed {
                            file,
                            error: "No audio.".into(),
                        }
                    } else {
                        TranscriptResult::Done {
                            transcript: transcript(&file, "so today we are testing three lenses"),
                            text_file: format!("{file}.txt"),
                            json_file: format!("{file}.json"),
                            file,
                        }
                    }
                })
                .collect()
        }
    }

    #[derive(Default)]
    struct FakeMeasure {
        measured: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl MeasureVideo for FakeMeasure {
        async fn measure(
            &self,
            _: &Path,
            file: &Path,
            sheet_file: &Path,
            _: Option<&CancellationToken>,
        ) -> anyhow::Result<Study> {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            self.measured.lock().unwrap().push(name.clone());
            if name == "broken.mp4" {
                bail!("ffmpeg couldn't read it.");
            }
            std::fs::write(sheet_file, "jpeg")?;
            let stat = std::fs::metadata(file)?;
            Ok(Study {
                version: STUDY_VERSION,
                file: file.to_string_lossy().into_owned(),
                size: stat.len(),
                modified: js::mtime_ms(&stat),
                duration: 30.0,
                width: Some(1080),
                height: Some(1920),
                aspect: Some("9:16".into()),
                fps: Some(30.0),
                loudness_lufs: Some(-14.0),
                shots: None,
                sound: None,
                sheet: Some(ContactSheet {
                    file: sheet_file.to_string_lossy().into_owned(),
                    frames: 24,
                    step_seconds: 1.25,
                }),
            })
        }
    }

    struct Setup {
        root: tempfile::TempDir,
        videos: PathBuf,
        study: StudyService,
        measure: Arc<FakeMeasure>,
    }

    fn setup() -> Setup {
        let root = temp();
        let videos = root.path().join("videos");
        std::fs::create_dir_all(videos.join("inside")).unwrap();
        for name in ["b.mp4", "a.MOV", "notes.txt", ".hidden.mp4", "inside/c.mp4"] {
            std::fs::write(videos.join(name), name).unwrap();
        }
        let measure = Arc::new(FakeMeasure::default());
        let study = StudyService::new(StudyOptions {
            dir: root.path().join("cache"),
            ffmpeg: Arc::new(|| Ok(PathBuf::from("ffmpeg"))),
            transcribe: Arc::new(FakeTranscribe),
            measure: Some(measure.clone()),
        });
        Setup {
            root,
            videos,
            study,
            measure,
        }
    }

    fn input(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn base(file: &str) -> String {
        Path::new(file)
            .file_name()
            .unwrap()
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn lists_a_folders_videos_in_name_order_one_level_deep() {
        let s = setup();
        let videos = s.videos.to_string_lossy().into_owned();
        let names: Vec<_> = video_files(std::slice::from_ref(&videos))
            .unwrap()
            .iter()
            .map(|f| base(f))
            .collect();
        assert_eq!(names, ["a.MOV", "b.mp4"]);
        let gone = s.videos.join("gone").to_string_lossy().into_owned();
        assert!(
            video_files(&[gone])
                .unwrap_err()
                .to_string()
                .contains("no file or folder")
        );
        let around = s
            .videos
            .join("inside")
            .join("..")
            .join("inside")
            .join("..")
            .join("..")
            .join("cache");
        assert!(
            video_files(&[around.to_string_lossy().into_owned()])
                .unwrap_err()
                .to_string()
                .contains("no file or folder")
        );
    }

    #[tokio::test]
    async fn measures_each_video_once_shows_its_contact_sheet_and_says_what_a_failed_one_failed_on()
    {
        let s = setup();
        std::fs::write(s.videos.join("broken.mp4"), "x").unwrap();
        let videos = s.videos.to_string_lossy().into_owned();
        let first = s
            .study
            .run(STUDY_MEDIA, &input(json!({ "files": [videos] })), None)
            .await
            .unwrap();
        let result: Value = serde_json::from_str(&first.text).unwrap();
        let listed = result["videos"].as_array().unwrap();
        let names: Vec<_> = listed
            .iter()
            .map(|video| base(video["file"].as_str().unwrap()))
            .collect();
        assert_eq!(names, ["a.MOV", "b.mp4", "broken.mp4"]);
        assert_eq!(listed[0]["aspect"], "9:16");
        assert_eq!(listed[0]["duration"], 30);
        assert_eq!(
            listed[0]["contact_sheet"],
            json!({ "image": 1, "frames": 24, "step_seconds": 1.25 })
        );
        let speech = &listed[0]["speech"];
        assert_eq!(
            (
                &speech["words"],
                &speech["words_per_minute"],
                &speech["first_word_at_seconds"],
                &speech["opens_with"]
            ),
            (
                &json!(7),
                &json!(120),
                &json!(0),
                &json!("so today we are testing three lenses")
            )
        );
        // What a cached measurement is checked against stays out of the agent's context.
        assert!(listed[0].get("modified").is_none());
        assert_eq!(listed[1]["speech"], Value::Null);
        assert_eq!(listed[1]["transcript_problem"], "No audio.");
        assert_eq!(listed[1]["contact_sheet"]["image"], 2);
        assert_eq!(
            listed[2],
            json!({ "file": s.videos.join("broken.mp4").to_string_lossy(), "error": "ffmpeg couldn't read it." })
        );
        assert_eq!(first.images.len(), 2);

        // Asked again, only the one that failed is measured again; a changed file is too.
        s.study
            .run(
                STUDY_MEDIA,
                &input(json!({ "files": [videos], "transcribe": false })),
                None,
            )
            .await
            .unwrap();
        assert_eq!(
            *s.measure.measured.lock().unwrap(),
            ["a.MOV", "b.mp4", "broken.mp4", "broken.mp4"]
        );
        std::fs::write(s.videos.join("a.MOV"), "a longer file now").unwrap();
        let a = s.videos.join("a.MOV").to_string_lossy().into_owned();
        s.study
            .run(
                STUDY_MEDIA,
                &input(json!({ "files": [a], "transcribe": false })),
                None,
            )
            .await
            .unwrap();
        assert_eq!(s.measure.measured.lock().unwrap().last().unwrap(), "a.MOV");
    }

    #[tokio::test]
    async fn studies_eight_videos_a_call_and_names_the_rest() {
        let s = setup();
        let many = s.root.path().join("many");
        std::fs::create_dir(&many).unwrap();
        for i in 0..10 {
            std::fs::write(many.join(format!("v{i}.mp4")), "x").unwrap();
        }
        let many = many.to_string_lossy().into_owned();
        let output = s
            .study
            .run(
                STUDY_MEDIA,
                &input(json!({ "files": [many], "transcribe": false })),
                None,
            )
            .await
            .unwrap();
        let result: Value = serde_json::from_str(&output.text).unwrap();
        assert_eq!(result["videos"].as_array().unwrap().len(), 8);
        let left: Vec<_> = result["not_studied_yet"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| base(f.as_str().unwrap()))
            .collect();
        assert_eq!(left, ["v8.mp4", "v9.mp4"]);
    }

    #[tokio::test]
    async fn compares_a_finished_video_with_its_raw_footage_and_says_what_it_needs_when_it_cant() {
        let s = setup();
        let path = |name: &str| s.videos.join(name).to_string_lossy().into_owned();
        let compared = s
            .study
            .run(
                COMPARE_CUT,
                &input(json!({ "final": path("a.MOV"), "raw": [path("inside")] })),
                None,
            )
            .await
            .unwrap();
        let compared: Value = serde_json::from_str(&compared.text).unwrap();
        assert_eq!(compared["kept_share"], 1);
        let failed = s
            .study
            .run(
                COMPARE_CUT,
                &input(json!({ "final": path("b.mp4"), "raw": [path("inside")] })),
                None,
            )
            .await;
        assert!(
            failed
                .unwrap_err()
                .to_string()
                .contains("Couldn't transcribe")
        );
        let no_raw = s
            .study
            .run(
                COMPARE_CUT,
                &input(json!({ "final": path("a.MOV"), "raw": [] })),
                None,
            )
            .await;
        assert!(no_raw.unwrap_err().to_string().contains("raw footage"));
    }
}
