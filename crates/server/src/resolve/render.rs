// Rendering a timeline through Resolve, for the preview panel and the agents' render_timeline tool. It runs
// in steps, each a script of its own (Resolve's MCP stops scripts after 60 seconds): start, poll until done
// or stopped, then put Resolve's timeline, page and render settings back as they were.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::connection::ResolveConnection;
use super::versions::TimelineRef;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RenderRequest {
    /// None renders the timeline open in Resolve.
    pub timeline_id: Option<String>,
    pub dir: PathBuf,
    /// File name without extension. Defaults to the timeline's name.
    pub name: Option<String>,
    pub preset: Option<String>,
    pub format: Option<String>,
    pub codec: Option<String>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// Scales down to at most this width, keeping the aspect ratio.
    pub max_width: Option<f64>,
    pub from_frame: Option<f64>,
    pub to_frame: Option<f64>,
    pub audio: Option<bool>,
    pub video: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum RenderEvent {
    Start {
        timeline: TimelineRef,
    },
    Progress {
        percent: f64,
    },
    Done {
        file: PathBuf,
        timeline: TimelineRef,
    },
    Error {
        message: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        stopped: Option<bool>,
    },
}

impl RenderEvent {
    fn error(message: impl Into<String>) -> Self {
        Self::Error {
            message: message.into(),
            stopped: None,
        }
    }
}

pub type OnRenderEvent = Arc<dyn Fn(RenderEvent) + Send + Sync>;

#[derive(Debug, Deserialize)]
struct Started {
    timeline: TimelineRef,
    target_dir: PathBuf,
    file_name: String,
    /// What render_finish.py needs to put Resolve back as it was; has "job" and "page".
    restore: Map<String, Value>,
}

#[derive(Debug, Deserialize)]
struct Poll {
    rendering: bool,
    percent: f64,
    status: Option<String>,
}

#[derive(Debug, Deserialize)]
struct Page {
    page: Option<String>,
}

/// The rendered file: Resolve names it in the job, adding the extension; look for it when it doesn't.
fn output_file(started: &Started) -> Option<PathBuf> {
    let named = started.target_dir.join(&started.file_name);
    if named.is_file() {
        return Some(named);
    }
    let stem = stem_of(&started.file_name);
    std::fs::read_dir(&started.target_dir)
        .ok()?
        .flatten()
        .find(|entry| stem_of(&entry.file_name().to_string_lossy()) == stem)
        .map(|entry| started.target_dir.join(entry.file_name()))
}

/// path.parse(name).name.
fn stem_of(name: &str) -> String {
    Path::new(name)
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// A render under way: stop it, or wait for its last event.
pub struct RenderJob {
    stop: CancellationToken,
    finished: JoinHandle<RenderEvent>,
}

impl RenderJob {
    /// Stops the render at its next check; Resolve is still put back.
    pub fn stop(&self) {
        self.stop.cancel();
    }

    /// Cancelling this stops the render, like stop().
    pub fn stop_token(&self) -> CancellationToken {
        self.stop.clone()
    }

    /// The last event: "done" or "error".
    pub async fn finished(self) -> RenderEvent {
        self.finished
            .await
            .unwrap_or_else(|error| RenderEvent::error(format!("The render failed: {error}")))
    }
}

fn opt<T: Serialize>(value: &Option<T>) -> Value {
    value.as_ref().map_or(Value::Null, |value| json!(value))
}

/// Renders one timeline. Events end with "done" or "error"; finished() resolves with that last event.
pub fn render_timeline(
    connection: Arc<dyn ResolveConnection>,
    request: RenderRequest,
    on_event: OnRenderEvent,
    poll: Duration,
) -> RenderJob {
    let stop = CancellationToken::new();
    let stopping = stop.clone();
    let finished = tokio::spawn(async move {
        let event = render(connection.as_ref(), &request, &on_event, poll, &stopping).await;
        on_event(event.clone());
        event
    });
    RenderJob { stop, finished }
}

async fn render(
    connection: &dyn ResolveConnection,
    request: &RenderRequest,
    on_event: &OnRenderEvent,
    poll: Duration,
    stopping: &CancellationToken,
) -> RenderEvent {
    let started = async {
        std::fs::create_dir_all(&request.dir)?;
        // Left-out values go as null, which the script reads as not given (JSON.stringify drops undefined).
        let mut args = json!({
            "dir": request.dir.to_string_lossy(),
            "name": opt(&request.name),
            "timeline_id": opt(&request.timeline_id),
            "preset": opt(&request.preset),
            "format": opt(&request.format),
            "codec": opt(&request.codec),
            "width": opt(&request.width),
            "height": opt(&request.height),
            "max_width": opt(&request.max_width),
            "from_frame": opt(&request.from_frame),
            "to_frame": opt(&request.to_frame),
            "audio": opt(&request.audio),
            "video": opt(&request.video),
        });
        // As JSON.stringify drops undefined, except timeline_id, which the TypeScript sent as null.
        if let Value::Object(args) = &mut args {
            args.retain(|key, value| key == "timeline_id" || !value.is_null());
        }
        connection
            .run_script::<Started>("render_start", args, 30)
            .await
    }
    .await;
    let started = match started {
        Ok(started) => started,
        Err(error) => return RenderEvent::error(error.to_string()),
    };
    on_event(RenderEvent::Start {
        timeline: started.timeline.clone(),
    });
    let job = started.restore.get("job").cloned().unwrap_or(Value::Null);
    let polled = async {
        let mut last = -1.0;
        let mut status: Option<String> = None;
        loop {
            if stopping.is_cancelled() {
                connection
                    .run_script_value("render_stop", json!({}), 45)
                    .await?;
                break;
            }
            let poll_result: Poll = connection
                .run_script("render_poll", json!({ "job": job }), 10)
                .await?;
            status = poll_result.status;
            if !poll_result.rendering {
                break;
            }
            if poll_result.percent != last {
                last = poll_result.percent;
                on_event(RenderEvent::Progress { percent: last });
            }
            tokio::time::sleep(poll).await;
        }
        anyhow::Ok(status)
    }
    .await;
    let result = match polled {
        Err(error) => RenderEvent::error(error.to_string()),
        Ok(status) => {
            let file = output_file(&started);
            if stopping.is_cancelled() {
                if let Some(file) = file {
                    let _ = std::fs::remove_file(file);
                }
                RenderEvent::Error {
                    message: "Render stopped.".into(),
                    stopped: Some(true),
                }
            } else {
                match (status.as_deref(), file) {
                    (Some("Complete"), Some(file)) => RenderEvent::Done {
                        file,
                        timeline: started.timeline.clone(),
                    },
                    (status, _) => RenderEvent::error(format!(
                        "Resolve's render ended with status {}.",
                        status.unwrap_or("unknown")
                    )),
                }
            }
        }
    };
    // Only once Resolve is put back may the app let an agent at it again.
    let put_back = async {
        connection
            .run_script_value("render_finish", json!({ "restore": started.restore }), 30)
            .await?;
        // Resolve opens the Deliver page by itself as a render ends, and ignores a switch back made too soon.
        if let Some(Value::String(page)) = started.restore.get("page") {
            for attempt in 0..10 {
                let now: Page = connection
                    .run_script("render_page", json!({ "page": page }), 10)
                    .await?;
                if now.page.as_deref() == Some(page.as_str()) && attempt > 0 {
                    break;
                }
                tokio::time::sleep(poll).await;
            }
        }
        anyhow::Ok(())
    }
    .await;
    if let Err(error) = put_back {
        tracing::error!("Couldn't put Resolve back after rendering: {error}");
    }
    result
}
