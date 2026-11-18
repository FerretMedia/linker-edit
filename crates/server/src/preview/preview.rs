// A small MP4 of a timeline for the preview panel, rendered by Resolve.

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use regex::Regex;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use crate::resolve::connection::ResolveConnection;
use crate::resolve::render::{RenderEvent, RenderRequest, render_timeline};
use crate::resolve::versions::TimelineRef;

/// What a render reports as it goes.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum PreviewEvent {
    Start {
        timeline: TimelineRef,
    },
    Progress {
        percent: f64,
    },
    Done {
        file: PathBuf,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stopped: Option<bool>,
    },
}

pub type OnPreviewEvent = Arc<dyn Fn(PreviewEvent) + Send + Sync>;

/// A preview render under way.
pub struct PreviewJob {
    stop: CancellationToken,
}

impl PreviewJob {
    /// A render that stop() stops by cancelling stop, for renderers other than Resolve's, as tests have.
    pub fn new(stop: CancellationToken) -> Self {
        PreviewJob { stop }
    }

    pub fn stop(&self) {
        self.stop.cancel();
    }
}

pub trait PreviewRenderer: Send + Sync {
    /// Renders a timeline, or Resolve's current one, calling back until a done or error event.
    fn render(&self, timeline_id: Option<String>, on_event: OnPreviewEvent) -> PreviewJob;
    /// The rendered preview with this file name, or None.
    fn file(&self, name: &str) -> Option<PathBuf>;
}

static PREVIEW_FILE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^preview-\d+\.mp4$").unwrap());

pub struct ResolvePreviewRenderer {
    dir: PathBuf,
    connection: Arc<dyn ResolveConnection>,
    poll: Duration,
}

/// Previews rendered by Resolve into dir; poll is how often a render is checked on (500 ms in the app).
pub fn create_preview_renderer(
    dir: PathBuf,
    connection: Arc<dyn ResolveConnection>,
    poll: Duration,
) -> ResolvePreviewRenderer {
    ResolvePreviewRenderer {
        dir,
        connection,
        poll,
    }
}

/// One preview at a time: the last one goes once a new render starts.
fn clear_previews(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if PREVIEW_FILE.is_match(&entry.file_name().to_string_lossy()) {
            let _ = std::fs::remove_file(entry.path());
        }
    }
    Ok(())
}

impl PreviewRenderer for ResolvePreviewRenderer {
    fn render(&self, timeline_id: Option<String>, on_event: OnPreviewEvent) -> PreviewJob {
        if let Err(error) = clear_previews(&self.dir) {
            // After render() returns, as the TypeScript's queueMicrotask.
            tokio::spawn(async move {
                on_event(PreviewEvent::Error {
                    message: error.to_string(),
                    stopped: None,
                })
            });
            return PreviewJob {
                stop: CancellationToken::new(),
            };
        }
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis();
        let request = RenderRequest {
            timeline_id,
            dir: self.dir.clone(),
            name: Some(format!("preview-{millis}")),
            format: Some("mp4".into()),
            max_width: Some(960.0),
            audio: Some(true),
            video: Some(true),
            ..Default::default()
        };
        let forward = Arc::new(move |event: RenderEvent| {
            on_event(match event {
                RenderEvent::Done { file, .. } => PreviewEvent::Done { file },
                RenderEvent::Error {
                    stopped: Some(true),
                    ..
                } => PreviewEvent::Error {
                    message: "Preview stopped.".into(),
                    stopped: Some(true),
                },
                RenderEvent::Error { message, stopped } => PreviewEvent::Error { message, stopped },
                RenderEvent::Start { timeline } => PreviewEvent::Start { timeline },
                RenderEvent::Progress { percent } => PreviewEvent::Progress { percent },
            })
        });
        let job = render_timeline(self.connection.clone(), request, forward, self.poll);
        PreviewJob {
            stop: job.stop_token(),
        }
    }

    fn file(&self, name: &str) -> Option<PathBuf> {
        let file = self.dir.join(name);
        (PREVIEW_FILE.is_match(name) && file.exists()).then_some(file)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use serde_json::{Value, json};
    use tokio::sync::mpsc;

    use super::*;
    use crate::resolve::connection::{ResolveError, ResolveErrorKind};
    use crate::resolve::testing::fake_connection;

    fn timeline() -> TimelineRef {
        TimelineRef {
            id: "t1".into(),
            name: "AI - Cut".into(),
        }
    }

    struct Rendered {
        events: Vec<PreviewEvent>,
        scripts: Vec<String>,
        _dir: tempfile::TempDir,
    }

    type Script = dyn Fn(&str, &Value) -> anyhow::Result<Value> + Send + Sync;

    async fn render(script: Arc<Script>, stop_after_start: bool) -> Rendered {
        let dir = tempfile::Builder::new()
            .prefix("linker-edit-preview-")
            .tempdir()
            .unwrap();
        let connection = Arc::new(fake_connection().script(move |name, args| script(name, args)));
        let renderer = create_preview_renderer(
            dir.path().to_path_buf(),
            connection.clone(),
            Duration::from_millis(1),
        );
        let (sender, mut receiver) = mpsc::unbounded_channel();
        let job: Arc<Mutex<Option<PreviewJob>>> = Arc::new(Mutex::new(None));
        let started = renderer.render(
            Some("t1".into()),
            Arc::new(move |event| sender.send(event).unwrap()),
        );
        *job.lock().unwrap() = Some(started);
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            if matches!(event, PreviewEvent::Start { .. }) && stop_after_start {
                job.lock().unwrap().as_ref().unwrap().stop();
            }
            let last = matches!(
                event,
                PreviewEvent::Done { .. } | PreviewEvent::Error { .. }
            );
            events.push(event);
            if last {
                break;
            }
        }
        Rendered {
            events,
            scripts: connection.script_names(),
            _dir: dir,
        }
    }

    fn started(args: &Value) -> Value {
        let dir = args["dir"].as_str().unwrap();
        let name = args["name"].as_str().unwrap();
        json!({ "timeline": timeline(), "target_dir": dir, "file_name": format!("{name}.mp4"), "restore": { "job": "j1", "page": "edit" } })
    }

    fn last_three(scripts: &[String]) -> &[String] {
        &scripts[scripts.len() - 3..]
    }

    #[tokio::test]
    async fn starts_reports_progress_finds_the_file_and_puts_resolve_back() {
        let polls = Arc::new(Mutex::new(0));
        let file = Arc::new(Mutex::new(PathBuf::new()));
        let seen = file.clone();
        let result = render(
            Arc::new(move |name, args| {
                if name == "render_start" {
                    let out = started(args);
                    *seen.lock().unwrap() = Path::new(out["target_dir"].as_str().unwrap()).join(out["file_name"].as_str().unwrap());
                    return Ok(out);
                }
                if name == "render_poll" {
                    let mut polls = polls.lock().unwrap();
                    *polls += 1;
                    if *polls < 3 {
                        return Ok(json!({ "rendering": true, "percent": *polls * 40, "status": "Rendering" }));
                    }
                    std::fs::write(&*seen.lock().unwrap(), "mp4").unwrap();
                    return Ok(json!({ "rendering": false, "percent": 100, "status": "Complete" }));
                }
                Ok(json!({ "page": "edit" }))
            }),
            false,
        )
        .await;
        let file = file.lock().unwrap().clone();
        assert_eq!(
            result.events,
            vec![
                PreviewEvent::Start {
                    timeline: timeline()
                },
                PreviewEvent::Progress { percent: 40.0 },
                PreviewEvent::Progress { percent: 80.0 },
                PreviewEvent::Done { file },
            ]
        );
        assert_eq!(
            last_three(&result.scripts),
            ["render_finish", "render_page", "render_page"]
        );
    }

    fn rendering(name: &str, args: &Value, poll: Value) -> anyhow::Result<Value> {
        Ok(match name {
            "render_start" => started(args),
            "render_poll" => poll,
            _ => json!({ "page": "edit" }),
        })
    }

    #[tokio::test]
    async fn stops_removes_the_partial_file_and_still_puts_resolve_back() {
        let result = render(
            Arc::new(|name, args| {
                rendering(
                    name,
                    args,
                    json!({ "rendering": true, "percent": 5, "status": "Rendering" }),
                )
            }),
            true,
        )
        .await;
        assert_eq!(
            result.events.last(),
            Some(&PreviewEvent::Error {
                message: "Preview stopped.".into(),
                stopped: Some(true)
            })
        );
        assert!(result.scripts.contains(&"render_stop".to_string()));
        // The last event comes once Resolve is put back.
        assert_eq!(
            last_three(&result.scripts),
            ["render_finish", "render_page", "render_page"]
        );
    }

    #[tokio::test]
    async fn passes_on_why_resolve_couldnt_start() {
        let result = render(
            Arc::new(|_, _| {
                Err(
                    ResolveError::new(ResolveErrorKind::Refused, "Resolve has no timeline open.")
                        .into(),
                )
            }),
            false,
        )
        .await;
        assert_eq!(
            result.events,
            vec![PreviewEvent::Error {
                message: "Resolve has no timeline open.".into(),
                stopped: None
            }]
        );
        assert_eq!(result.scripts, ["render_start"]);
    }

    #[tokio::test]
    async fn reports_a_render_that_didnt_complete() {
        let result = render(
            Arc::new(|name, args| {
                rendering(
                    name,
                    args,
                    json!({ "rendering": false, "percent": 20, "status": "Failed" }),
                )
            }),
            false,
        )
        .await;
        assert_eq!(
            result.events.last(),
            Some(&PreviewEvent::Error {
                message: "Resolve's render ended with status Failed.".into(),
                stopped: None
            })
        );
    }

    #[tokio::test]
    async fn serves_only_preview_files() {
        let dir = tempfile::tempdir().unwrap();
        let renderer = create_preview_renderer(
            dir.path().to_path_buf(),
            Arc::new(fake_connection()),
            Duration::from_millis(1),
        );
        std::fs::write(dir.path().join("preview-1.mp4"), "mp4").unwrap();
        std::fs::write(dir.path().join("notes.txt"), "").unwrap();
        assert_eq!(
            renderer.file("preview-1.mp4"),
            Some(dir.path().join("preview-1.mp4"))
        );
        assert_eq!(renderer.file("notes.txt"), None);
        assert_eq!(renderer.file("../preview-1.mp4"), None);
    }
}
