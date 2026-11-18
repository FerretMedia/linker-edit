// Speech to text for the agents' transcribe tool: one file at a time, since they share the GPU, and each
// transcript kept until its file changes.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use async_trait::async_trait;
use linker_edit_protocol::TranscriptionStatus;
use regex::Regex;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use super::tools::{SetupProgress, WhisperTools, prepare_tools};
use super::whisper::{Transcript, transcribe_file};
use crate::js::{self, Serial, Shared};

/// Finds or downloads the engine.
#[async_trait]
pub trait PrepareTools: Send + Sync {
    async fn prepare(
        &self,
        on_progress: SetupProgress,
        cancel: CancellationToken,
    ) -> anyhow::Result<WhisperTools>;
}

/// The engine Linker-Edit downloads into the tools folder (config.tools_dir), or the user's own whisper-cli
/// (config.whisper_cli): prepare_tools.
pub struct DownloadTools {
    pub dir: PathBuf,
    pub whisper_cli: Option<PathBuf>,
}

#[async_trait]
impl PrepareTools for DownloadTools {
    async fn prepare(
        &self,
        on_progress: SetupProgress,
        cancel: CancellationToken,
    ) -> anyhow::Result<WhisperTools> {
        prepare_tools(&self.dir, self.whisper_cli.as_deref(), on_progress, cancel).await
    }
}

/// Transcribes one file with the prepared engine.
#[async_trait]
pub trait TranscribeFile: Send + Sync {
    async fn transcribe_file(
        &self,
        tools: &WhisperTools,
        file: &Path,
        language: &str,
    ) -> anyhow::Result<Transcript>;
}

/// whisper.cpp itself: whisper::transcribe_file.
pub struct Whisper;

#[async_trait]
impl TranscribeFile for Whisper {
    async fn transcribe_file(
        &self,
        tools: &WhisperTools,
        file: &Path,
        language: &str,
    ) -> anyhow::Result<Transcript> {
        transcribe_file(tools, file, language).await
    }
}

pub struct TranscriberOptions {
    /// Where transcripts are kept, one .json (with word times) and one .txt per media file.
    pub dir: PathBuf,
    /// Finds or downloads the engine; called once, again only after it failed.
    pub prepare: Arc<dyn PrepareTools>,
    /// Whether the engine is already downloaded, checked without downloading (tools_ready); unknown when None.
    pub ready: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Whisper when None.
    pub transcribe_file: Option<Arc<dyn TranscribeFile>>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TranscriptResult {
    Done {
        file: String,
        transcript: Transcript,
        text_file: String,
        json_file: String,
    },
    Failed {
        file: String,
        error: String,
    },
}

impl TranscriptResult {
    pub fn file(&self) -> &str {
        match self {
            Self::Done { file, .. } | Self::Failed { file, .. } => file,
        }
    }
}

fn normalize(file: &str) -> String {
    let resolved = js::resolve(file).to_string_lossy().into_owned();
    if cfg!(windows) {
        resolved.to_lowercase()
    } else {
        resolved
    }
}

fn seconds(value: f64) -> String {
    let minutes = (value / 60.0).floor();
    format!("{minutes}:{:0>2}", js::math_round(value % 60.0))
}

/// The transcript as agents read it: one line per segment, with times in seconds.
pub fn transcript_text(transcript: &Transcript) -> String {
    let mut lines = vec![
        transcript.file.clone(),
        format!(
            "Language: {}. Length: {}. Engine: {}.",
            transcript.language,
            seconds(transcript.duration),
            transcript.engine
        ),
        "Times are seconds from the start of the source file.".to_string(),
        String::new(),
    ];
    lines.extend(transcript.segments.iter().map(|segment| {
        format!(
            "[{}-{}] {}",
            js::to_fixed(segment.start, 2),
            js::to_fixed(segment.end, 2),
            segment.text
        )
    }));
    if transcript.segments.is_empty() {
        lines.push("(no speech found)".into());
    }
    format!("{}\n", lines.join("\n"))
}

/// Transcripts go inline while they fit in a tool result (budget counts UTF-16 units, as JavaScript's length
/// did); longer ones are left in their files.
pub fn describe_results(results: &[TranscriptResult], budget: usize) -> String {
    let mut parts = Vec::new();
    let mut left = budget;
    for result in results {
        match result {
            TranscriptResult::Failed { file, error } => {
                parts.push(format!("{file}\nCouldn't transcribe: {error}"));
            }
            TranscriptResult::Done {
                file,
                transcript,
                text_file,
                json_file,
            } => {
                let text = transcript_text(transcript);
                let files = format!("Transcript: {text_file}\nWord timings: {json_file}");
                let length = text.encode_utf16().count();
                if length <= left {
                    left -= length;
                    parts.push(format!("{text}{files}"));
                } else {
                    parts.push(format!(
                        "{file}\nLanguage: {}, {} segments, too long to show here.\n{files}",
                        transcript.language,
                        transcript.segments.len()
                    ));
                }
            }
        }
    }
    parts.join("\n\n---\n\n")
}

/// describe_results' budget in the TypeScript.
pub const DESCRIBE_BUDGET: usize = 40_000;

type StatusListener = Arc<dyn Fn(&TranscriptionStatus) + Send + Sync>;
type Prepared = Result<WhisperTools, String>;

struct State {
    // Known once checked or prepared; None while nothing says.
    ready: Option<bool>,
    status: TranscriptionStatus,
    tools: Option<watch::Receiver<Option<Prepared>>>,
    setup_abort: Option<(u64, CancellationToken)>,
    done: u32,
    total: u32,
    pending: HashMap<String, Shared<TranscriptResult>>,
}

struct Inner {
    options: TranscriberOptions,
    state: Mutex<State>,
    listeners: Mutex<Vec<(u64, StatusListener)>>,
    next_id: AtomicU64,
    queue: Serial,
}

/// Transcribes media files one at a time, since they share the GPU, and keeps each transcript until its file
/// changes. Cheap to clone; all clones are the same transcriber.
#[derive(Clone)]
pub struct Transcriber {
    inner: Arc<Inner>,
}

fn idle(ready: Option<bool>) -> TranscriptionStatus {
    TranscriptionStatus::Idle { ready }
}

impl Inner {
    fn set_status(&self, next: TranscriptionStatus) {
        self.state.lock().unwrap().status = next.clone();
        let listeners: Vec<_> = self
            .listeners
            .lock()
            .unwrap()
            .iter()
            .map(|(_, listener)| listener.clone())
            .collect();
        for listener in listeners {
            listener(&next);
        }
    }

    fn set_idle(&self) {
        let ready = self.state.lock().unwrap().ready;
        self.set_status(idle(ready));
    }

    /// Starts preparing when nothing is, and gives what to wait on.
    fn prepare(self: &Arc<Self>) -> watch::Receiver<Option<Prepared>> {
        let mut state = self.state.lock().unwrap();
        if let Some(tools) = &state.tools {
            return tools.clone();
        }
        let cancel = CancellationToken::new();
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        state.setup_abort = Some((id, cancel.clone()));
        let (sender, receiver) = watch::channel(None);
        state.tools = Some(receiver.clone());
        drop(state);
        let inner = self.clone();
        tokio::spawn(async move {
            let progress = Arc::downgrade(&inner);
            let on_progress: SetupProgress = Arc::new(move |step, percent| {
                if let Some(inner) = progress.upgrade() {
                    inner.set_status(TranscriptionStatus::Preparing {
                        step: step.to_string(),
                        percent,
                    });
                }
            });
            let result = inner
                .options
                .prepare
                .prepare(on_progress, cancel.clone())
                .await;
            let mut state = inner.state.lock().unwrap();
            let result = match result {
                Ok(tools) => {
                    state.ready = Some(true);
                    Ok(tools)
                }
                Err(error) => {
                    state.tools = None;
                    state.ready = inner
                        .options
                        .ready
                        .as_ref()
                        .map(|ready| ready())
                        .or(state.ready);
                    Err(if cancel.is_cancelled() {
                        "Setting up transcription was cancelled.".to_string()
                    } else {
                        error.to_string()
                    })
                }
            };
            if state
                .setup_abort
                .as_ref()
                .is_some_and(|(setup, _)| *setup == id)
            {
                state.setup_abort = None;
            }
            drop(state);
            sender.send_replace(Some(result));
        });
        receiver
    }

    async fn prepared(self: &Arc<Self>) -> Prepared {
        let mut tools = self.prepare();
        let result = tools.wait_for(Option::is_some).await;
        match result {
            Ok(result) => result.clone().unwrap_or_else(|| Err("no result".into())),
            Err(_) => Err("Setting up transcription stopped.".into()),
        }
    }

    fn cache_files(&self, file: &str) -> (String, String) {
        static UNSAFE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^A-Za-z0-9_.-]+").unwrap());
        let key = &js::sha256_hex(&normalize(file))[..12];
        let name = js::parsed_name(Path::new(file));
        let name = UNSAFE.replace_all(&name, "_");
        let base = self
            .options
            .dir
            .join(format!("{}-{key}", js::slice(&name, 60)));
        let base = base.to_string_lossy();
        (format!("{base}.json"), format!("{base}.txt"))
    }

    fn cached(&self, file: &str, language: &str) -> Option<Transcript> {
        let (json_file, _) = self.cache_files(file);
        let transcript: Transcript =
            serde_json::from_str(&std::fs::read_to_string(json_file).ok()?).ok()?;
        let stat = std::fs::metadata(file).ok()?;
        if transcript.size != stat.len() || transcript.modified != js::mtime_ms(&stat) {
            return None;
        }
        if language != "auto" && transcript.language != language {
            return None;
        }
        Some(transcript)
    }

    fn save(&self, file: &str, transcript: Transcript) -> anyhow::Result<TranscriptResult> {
        let (json_file, text_file) = self.cache_files(file);
        std::fs::create_dir_all(&self.options.dir)?;
        std::fs::write(&json_file, serde_json::to_string(&transcript)?)?;
        std::fs::write(&text_file, transcript_text(&transcript))?;
        Ok(TranscriptResult::Done {
            file: file.to_string(),
            transcript,
            text_file,
            json_file,
        })
    }

    async fn run(self: Arc<Self>, file: String, language: String, key: String) -> TranscriptResult {
        let result = match self.prepared().await {
            Ok(tools) => {
                let (done, total) = {
                    let state = self.state.lock().unwrap();
                    (state.done, state.total)
                };
                let name = Path::new(&file)
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_default();
                self.set_status(TranscriptionStatus::Transcribing {
                    file: name,
                    done,
                    total,
                });
                let transcriber: Arc<dyn TranscribeFile> = self
                    .options
                    .transcribe_file
                    .clone()
                    .unwrap_or_else(|| Arc::new(Whisper));
                transcriber
                    .transcribe_file(&tools, Path::new(&file), &language)
                    .await
                    .and_then(|transcript| self.save(&file, transcript))
                    .unwrap_or_else(|error| TranscriptResult::Failed {
                        file: file.clone(),
                        error: error.to_string(),
                    })
            }
            Err(error) => TranscriptResult::Failed {
                file: file.clone(),
                error,
            },
        };
        let all_done = {
            let mut state = self.state.lock().unwrap();
            state.pending.remove(&key);
            state.done += 1;
            let all_done = state.done == state.total;
            if all_done {
                state.done = 0;
                state.total = 0;
            }
            all_done
        };
        if all_done {
            self.set_idle();
        }
        result
    }

    fn transcribe_one(
        self: &Arc<Self>,
        file: String,
        language: String,
    ) -> Shared<TranscriptResult> {
        let ready = |result| {
            let (sender, receiver) = watch::channel(Some(result));
            drop(sender);
            Shared::from_receiver(receiver)
        };
        let key = format!("{}\n{language}", normalize(&file));
        let mut state = self.state.lock().unwrap();
        if let Some(running) = state.pending.get(&key) {
            return running.clone();
        }
        if !std::fs::metadata(&file).is_ok_and(|stat| stat.is_file()) {
            return ready(TranscriptResult::Failed {
                file,
                error: "File not found.".into(),
            });
        }
        if let Some(hit) = self.cached(&file, &language) {
            let (json_file, text_file) = self.cache_files(&file);
            return ready(TranscriptResult::Done {
                file,
                transcript: hit,
                text_file,
                json_file,
            });
        }
        state.total += 1;
        let job = Shared::queue(&self.queue, self.clone().run(file, language, key.clone()));
        state.pending.insert(key, job.clone());
        job
    }
}

impl Transcriber {
    pub fn new(options: TranscriberOptions) -> Self {
        let ready = options.ready.as_ref().map(|ready| ready());
        Self {
            inner: Arc::new(Inner {
                options,
                state: Mutex::new(State {
                    ready,
                    status: idle(ready),
                    tools: None,
                    setup_abort: None,
                    done: 0,
                    total: 0,
                    pending: HashMap::new(),
                }),
                listeners: Mutex::default(),
                next_id: AtomicU64::new(0),
                queue: Serial::default(),
            }),
        }
    }

    pub fn status(&self) -> TranscriptionStatus {
        self.inner.state.lock().unwrap().status.clone()
    }

    /// Downloads the engine now, rather than with the first transcription. Starts when called; the future waits
    /// for it. Needs a tokio runtime.
    pub fn setup(&self) -> impl Future<Output = anyhow::Result<()>> + Send + 'static {
        let inner = self.inner.clone();
        let transcribing = matches!(self.status(), TranscriptionStatus::Transcribing { .. });
        let started = (!transcribing).then(|| inner.prepare());
        async move {
            let Some(mut tools) = started else {
                return Ok(());
            };
            let result = tools
                .wait_for(Option::is_some)
                .await
                .ok()
                .and_then(|result| result.clone());
            // A transcription may have started meanwhile; it reports its own status.
            if !matches!(
                inner.state.lock().unwrap().status,
                TranscriptionStatus::Transcribing { .. }
            ) {
                inner.set_idle();
            }
            match result {
                Some(Ok(_)) => Ok(()),
                Some(Err(error)) => Err(anyhow::anyhow!(error)),
                None => Err(anyhow::anyhow!("Setting up transcription stopped.")),
            }
        }
    }

    /// Stops a download in progress; transcriptions waiting for it fail and say so.
    pub fn cancel_setup(&self) {
        if let Some((_, cancel)) = &self.inner.state.lock().unwrap().setup_abort {
            cancel.cancel();
        }
    }

    /// Calls the listener on every status change; the returned function stops that.
    pub fn on_status(
        &self,
        listener: impl Fn(&TranscriptionStatus) + Send + Sync + 'static,
    ) -> impl FnOnce() + Send + 'static {
        let id = self.inner.next_id.fetch_add(1, Ordering::SeqCst);
        self.inner
            .listeners
            .lock()
            .unwrap()
            .push((id, Arc::new(listener)));
        let inner = Arc::downgrade(&self.inner);
        move || {
            if let Some(inner) = inner.upgrade() {
                inner
                    .listeners
                    .lock()
                    .unwrap()
                    .retain(|(other, _)| *other != id);
            }
        }
    }

    /// `language` is a Whisper code such as "en" or "nl"; "auto" (or None) detects it per file. The files are
    /// queued when called; the future waits for their results, in the same order.
    pub fn transcribe(
        &self,
        files: &[String],
        language: Option<&str>,
    ) -> impl Future<Output = Vec<TranscriptResult>> + Send + 'static {
        let language = language.map(|language| js::trim(language).to_lowercase());
        let language = language
            .filter(|language| !language.is_empty())
            .unwrap_or_else(|| "auto".into());
        let jobs: Vec<(String, Shared<TranscriptResult>)> = files
            .iter()
            .map(|file| {
                let file = js::resolve(file).to_string_lossy().into_owned();
                (
                    file.clone(),
                    self.inner.transcribe_one(file, language.clone()),
                )
            })
            .collect();
        async move {
            let mut results = Vec::with_capacity(jobs.len());
            for (file, job) in jobs {
                results.push(job.get().await.unwrap_or(TranscriptResult::Failed {
                    file,
                    error: "Transcribing stopped.".into(),
                }));
            }
            results
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::AtomicUsize;

    use super::*;
    use crate::library::testing::temp;

    fn tools() -> WhisperTools {
        WhisperTools {
            ffmpeg: "ffmpeg".into(),
            whisper_cli: "whisper-cli".into(),
            model: "m".into(),
            vad_model: "v".into(),
            label: "test".into(),
        }
    }

    #[derive(Default)]
    struct Counting(AtomicUsize);

    #[async_trait]
    impl PrepareTools for Counting {
        async fn prepare(
            &self,
            _: SetupProgress,
            _: CancellationToken,
        ) -> anyhow::Result<WhisperTools> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Ok(tools())
        }
    }

    #[derive(Default)]
    struct Fake {
        runs: Mutex<Vec<String>>,
    }

    #[async_trait]
    impl TranscribeFile for Fake {
        async fn transcribe_file(
            &self,
            _: &WhisperTools,
            file: &Path,
            language: &str,
        ) -> anyhow::Result<Transcript> {
            let name = file.file_name().unwrap().to_string_lossy().into_owned();
            self.runs.lock().unwrap().push(format!("{name}:{language}"));
            if name == "broken.wav" {
                anyhow::bail!("Whisper failed");
            }
            let stat = std::fs::metadata(file)?;
            Ok(Transcript {
                file: file.to_string_lossy().into_owned(),
                size: stat.len(),
                modified: js::mtime_ms(&stat),
                engine: "test".into(),
                language: if language == "auto" {
                    "nl".into()
                } else {
                    language.into()
                },
                duration: 2.0,
                segments: vec![super::super::whisper::Segment {
                    start: 0.5,
                    end: 1.25,
                    text: "Hallo daar.".into(),
                    words: vec![],
                }],
            })
        }
    }

    struct Setup {
        root: tempfile::TempDir,
        prepare: Arc<Counting>,
        fake: Arc<Fake>,
        transcriber: Transcriber,
        statuses: Arc<Mutex<Vec<TranscriptionStatus>>>,
    }

    fn setup() -> Setup {
        let root = temp();
        let prepare = Arc::new(Counting::default());
        let fake = Arc::new(Fake::default());
        let transcriber = Transcriber::new(TranscriberOptions {
            dir: root.path().join("transcripts"),
            prepare: prepare.clone(),
            ready: None,
            transcribe_file: Some(fake.clone()),
        });
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let seen = statuses.clone();
        let _keep = transcriber.on_status(move |status| seen.lock().unwrap().push(status.clone()));
        Setup {
            root,
            prepare,
            fake,
            transcriber,
            statuses,
        }
    }

    impl Setup {
        fn media(&self, name: &str, contents: &str) -> String {
            let file = self.root.path().join(name);
            std::fs::write(&file, contents).unwrap();
            file.to_string_lossy().into_owned()
        }

        fn runs(&self) -> Vec<String> {
            self.fake.runs.lock().unwrap().clone()
        }
    }

    #[tokio::test]
    async fn keeps_a_transcript_until_its_file_or_the_asked_language_changes() {
        let s = setup();
        let file = s.media("interview.mov", "audio");
        let files = [file.clone()];
        s.transcriber.transcribe(&files, None).await;
        s.transcriber.transcribe(&files, None).await;
        s.transcriber.transcribe(&files, Some("nl")).await;
        assert_eq!(s.runs(), ["interview.mov:auto"]);

        s.transcriber.transcribe(&files, Some("en")).await;
        s.media("interview.mov", "a longer recording");
        s.transcriber.transcribe(&files, Some("en")).await;
        assert_eq!(
            s.runs(),
            ["interview.mov:auto", "interview.mov:en", "interview.mov:en"]
        );
        assert_eq!(s.prepare.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn transcribes_files_one_at_a_time_reports_each_failure_on_its_own_file_then_goes_idle() {
        let s = setup();
        let files = [
            s.media("a.wav", "audio"),
            s.media("broken.wav", "audio"),
            s.media("b.wav", "audio"),
            "missing.wav".to_string(),
        ];
        let results = s.transcriber.transcribe(&files, None).await;
        assert_eq!(s.runs(), ["a.wav:auto", "broken.wav:auto", "b.wav:auto"]);
        let outcomes: Vec<String> = results
            .iter()
            .map(|result| match result {
                TranscriptResult::Done { transcript, .. } => transcript.language.clone(),
                TranscriptResult::Failed { error, .. } => error.clone(),
            })
            .collect();
        assert_eq!(outcomes, ["nl", "Whisper failed", "nl", "File not found."]);
        let statuses = s.statuses.lock().unwrap().clone();
        let done: Vec<u32> = statuses
            .iter()
            .filter_map(|status| match status {
                TranscriptionStatus::Transcribing { done, .. } => Some(*done),
                _ => None,
            })
            .collect();
        assert_eq!(done, [0, 1, 2]);
        assert_eq!(
            statuses.last(),
            Some(&TranscriptionStatus::Idle { ready: Some(true) })
        );

        let text = describe_results(&results, DESCRIBE_BUDGET);
        assert!(text.contains("[0.50-1.25] Hallo daar."), "{text}");
        assert!(text.contains("Couldn't transcribe: Whisper failed"));
    }

    /// Reports its progress, then waits until cancelled, except the second time, which finishes.
    struct Slow {
        calls: AtomicUsize,
        downloaded: Arc<std::sync::atomic::AtomicBool>,
    }

    #[async_trait]
    impl PrepareTools for Slow {
        async fn prepare(
            &self,
            on_progress: SetupProgress,
            cancel: CancellationToken,
        ) -> anyhow::Result<WhisperTools> {
            let call = self.calls.fetch_add(1, Ordering::SeqCst);
            on_progress("Downloading the speech model", Some(10.0));
            if call > 0 {
                self.downloaded.store(true, Ordering::SeqCst);
                return Ok(tools());
            }
            cancel.cancelled().await;
            anyhow::bail!("aborted")
        }
    }

    #[tokio::test]
    async fn sets_up_ahead_of_the_first_transcription_and_a_cancelled_setup_says_so_and_can_start_again()
     {
        let root = temp();
        let file = root.path().join("a.wav");
        std::fs::write(&file, "audio").unwrap();
        let file = file.to_string_lossy().into_owned();
        let downloaded = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let slow = Arc::new(Slow {
            calls: AtomicUsize::new(0),
            downloaded: downloaded.clone(),
        });
        let is_downloaded = downloaded.clone();
        let transcriber = Transcriber::new(TranscriberOptions {
            dir: root.path().join("transcripts"),
            prepare: slow.clone(),
            ready: Some(Arc::new(move || is_downloaded.load(Ordering::SeqCst))),
            transcribe_file: Some(Arc::new(Fake::default())),
        });
        assert_eq!(
            transcriber.status(),
            TranscriptionStatus::Idle { ready: Some(false) }
        );

        let (preparing, mut seen) = watch::channel(false);
        let _keep = transcriber.on_status(move |status| {
            if matches!(status, TranscriptionStatus::Preparing { .. }) {
                preparing.send_replace(true);
            }
        });
        let first = transcriber.setup();
        let waiting = transcriber.transcribe(std::slice::from_ref(&file), None);
        seen.wait_for(|seen| *seen).await.unwrap();
        assert_eq!(
            transcriber.status(),
            TranscriptionStatus::Preparing {
                step: "Downloading the speech model".into(),
                percent: Some(10.0)
            }
        );
        transcriber.cancel_setup();
        assert_eq!(
            first.await.unwrap_err().to_string(),
            "Setting up transcription was cancelled."
        );
        assert_eq!(
            waiting.await,
            [TranscriptResult::Failed {
                file: file.clone(),
                error: "Setting up transcription was cancelled.".into()
            }]
        );
        assert_eq!(
            transcriber.status(),
            TranscriptionStatus::Idle { ready: Some(false) }
        );

        transcriber.setup().await.unwrap();
        assert_eq!(slow.calls.load(Ordering::SeqCst), 2);
        assert_eq!(
            transcriber.status(),
            TranscriptionStatus::Idle { ready: Some(true) }
        );
    }

    #[test]
    fn writes_times_as_javascript_did() {
        assert_eq!(seconds(125.4), "2:05");
        assert_eq!(seconds(59.6), "0:60");
    }
}
