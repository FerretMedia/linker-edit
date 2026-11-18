// The port of app.test.ts.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use linker_edit_protocol::{
    ApprovalMode, ChatItem, ContextUsage, HistoryEntry, PlayableTimeline, PreviewState, Provider,
    ResolveLink, ResolveLinkKind, ResolveStatus, Tone, ToolStatus,
};
use serde_json::{Value, json};
use tokio::sync::{Notify, oneshot};

use super::testing::*;
use super::*;
use crate::history::history::AgentTurn;
use crate::preview::preview::{OnPreviewEvent, PreviewEvent, PreviewJob, PreviewRenderer};
use crate::projects::store::LoadedProject;
use crate::projects::testing::{MemoryProjectStore, memory_project_store, workspace_project};
use crate::providers::types::ItemKind;
use crate::providers::types::ItemPatch;
use crate::resolve::versions::{Applied, TimelineRef as VersionRef};
use crate::resolve_projects::{OpenedProject, OpenedProjectName, ResolveProjects};
use crate::transcribe::tools::{SetupProgress, WhisperTools};
use crate::transcribe::transcriber::PrepareTools;

fn tool(id: &str, name: &str, input: Value, status: ToolStatus) -> ItemPatch {
    ItemPatch {
        id: id.into(),
        kind: Some(ItemKind::Tool),
        name: Some(name.into()),
        input: Some(input),
        status: Some(status),
        ..ItemPatch::default()
    }
}

fn assistant(id: &str, text: &str) -> ItemPatch {
    ItemPatch {
        id: id.into(),
        kind: Some(ItemKind::Assistant),
        text: Some(text.into()),
        ..ItemPatch::default()
    }
}

fn status(id: &str, status: ToolStatus) -> ItemPatch {
    ItemPatch {
        id: id.into(),
        status: Some(status),
        ..ItemPatch::default()
    }
}

/// A gate a scripted turn waits at until the test opens it.
#[derive(Clone)]
struct Gate(Arc<tokio::sync::watch::Sender<bool>>);

impl Default for Gate {
    fn default() -> Self {
        Gate(Arc::new(tokio::sync::watch::channel(false).0))
    }
}

impl Gate {
    fn open(&self) {
        self.0.send_replace(true);
    }
    async fn opened(&self) {
        let _ = self.0.subscribe().wait_for(|open| *open).await;
    }
}

mod turns {
    use super::*;

    #[tokio::test]
    async fn runs_a_turn_and_marks_tools_still_running_at_the_end_as_failed() {
        let h = setup()
            .scripts([script(|turn| async move {
                turn.sink
                    .upsert(tool("t1", "Read", json!({}), ToolStatus::Running));
                turn.sink.upsert(assistant("a1", "Done"));
                Ok(())
            })])
            .start();
        h.send(json!({ "type": "send", "text": "  hello  " }));
        assert!(h.state().meta.busy);
        h.turn_done().await;
        let items = h.state().items;
        assert!(matches!(&items[0], ChatItem::User { text, .. } if text == "hello"));
        assert!(
            matches!(&items[1], ChatItem::Tool { id, status: ToolStatus::Error, .. } if id == "t1")
        );
        assert!(
            matches!(&items[2], ChatItem::Assistant { id, text } if id == "a1" && text == "Done")
        );
        assert_eq!(items.len(), 3);
    }

    #[tokio::test]
    async fn shows_a_provider_error_but_not_one_caused_by_stop() {
        let h = setup()
            .scripts([
                script(|_| async { anyhow::bail!("CLI not logged in") }),
                script(|turn| async move {
                    turn.signal.cancelled().await;
                    anyhow::bail!("aborted")
                }),
            ])
            .start();
        h.send(json!({ "type": "send", "text": "one" }));
        h.turn_done().await;
        h.send(json!({ "type": "send", "text": "two" }));
        h.send(json!({ "type": "stop" }));
        h.turn_done().await;
        assert_eq!(
            notices(&h.state().items),
            [
                (Tone::Error, "CLI not logged in".to_string()),
                (Tone::Info, "Stopped.".to_string())
            ]
        );
    }

    #[tokio::test]
    async fn stops_a_turns_resolve_tool_with_it_and_keeps_resolve_taken_until_the_tool_has_wound_down()
     {
        let (signal_tx, signal_rx) = oneshot::channel();
        let signal_tx = Arc::new(Mutex::new(Some(signal_tx)));
        let finish = Arc::new(Notify::new());
        let finished = finish.clone();
        let h = setup()
            .with(move |options| {
                options.resolve_tools = Some(Arc::new(move |_, _, _, signal| {
                    if let Some(tx) = signal_tx.lock().unwrap().take() {
                        let _ = tx.send(signal);
                    }
                    let finished = finished.clone();
                    Box::pin(async move {
                        finished.notified().await;
                        Ok(ToolOutput {
                            text: "stopped".into(),
                            images: None,
                        })
                    })
                }));
            })
            .scripts([script(|turn| async move {
                let app = turn.app.clone();
                tokio::spawn(async move { app.resolve_tool("render_timeline", json!({})).await });
                turn.signal.cancelled().await;
                Ok(())
            })])
            .start();
        h.send(json!({ "type": "send", "text": "render it" }));
        let tool_signal = signal_rx.await.unwrap().expect("the turn's signal");
        h.send(json!({ "type": "stop" }));
        h.turn_done().await;
        assert!(tool_signal.is_cancelled());
        assert!(h.app.busy());
        assert!(
            h.state()
                .meta
                .blocked
                .unwrap()
                .message
                .contains("stopped turn")
        );
        assert!(
            h.app
                .resolve_tool("list_timelines", json!({}))
                .await
                .is_error
        );
        finish.notify_waiters();
        h.all_done().await;
        h.client.until(|state| state.meta.blocked.is_none()).await;
    }

    #[tokio::test]
    async fn ignores_a_send_while_a_turn_is_running() {
        let gate = Gate::default();
        let opened = gate.clone();
        let h = setup()
            .scripts([script(move |_| async move {
                opened.opened().await;
                Ok(())
            })])
            .start();
        h.send(json!({ "type": "send", "text": "first" }));
        h.send(json!({ "type": "send", "text": "second" }));
        gate.open();
        h.turn_done().await;
        assert_eq!(kinds(&h.state().items), ["first"]);
    }
}

fn reply(text: &str, is_error: bool) -> ToolReply {
    ToolReply {
        text: text.into(),
        images: None,
        is_error,
    }
}

type Answers = Arc<Mutex<Vec<bool>>>;

/// A turn waiting for the user to approve a new timeline; answers collects what the provider was told.
async fn awaiting_approval() -> (Harness, Answers) {
    let answers: Answers = Arc::default();
    let seen = answers.clone();
    let h = setup()
        .scripts([script(move |turn| async move {
            let allowed = turn
                .sink
                .request_approval("mcp__linker__create_timeline", &json!({ "name": "Cut" }))
                .await;
            seen.lock().unwrap().push(allowed);
            Ok(())
        })])
        .start();
    h.send(json!({ "type": "send", "text": "build it" }));
    h.client
        .until(|state| !state.meta.approvals.is_empty())
        .await;
    (h, answers)
}

fn settings() -> Arc<MemorySettings> {
    Arc::new(MemorySettings::default())
}

fn with_settings(settings: &Arc<MemorySettings>) -> impl FnOnce(&mut AppOptions) + 'static {
    let settings = settings.clone();
    move |options| options.settings = Some(settings)
}

mod approvals {
    use super::*;

    async fn passes_the_users_answer_to_the_provider(allow: bool) {
        let (h, answers) = awaiting_approval().await;
        let request = h.state().meta.approvals[0].clone();
        assert_eq!(request.tool, "mcp__linker__create_timeline");
        assert_eq!(request.input, json!({ "name": "Cut" }));
        h.send(json!({ "type": "approve", "id": request.id, "allow": allow }));
        h.turn_done().await;
        assert_eq!(*answers.lock().unwrap(), [allow]);
        assert!(h.state().meta.approvals.is_empty());
    }

    #[tokio::test]
    async fn passes_the_users_answer_true_to_the_provider() {
        passes_the_users_answer_to_the_provider(true).await;
    }

    #[tokio::test]
    async fn passes_the_users_answer_false_to_the_provider() {
        passes_the_users_answer_to_the_provider(false).await;
    }

    #[tokio::test]
    async fn runs_agents_resolve_tools_for_the_thread_working_in_resolve_asking_before_changes() {
        let results: Arc<Mutex<Vec<ToolReply>>> = Arc::default();
        let ran: Arc<Mutex<Vec<String>>> = Arc::default();
        let (seen, log) = (results.clone(), ran.clone());
        let h = setup()
            .with(move |options| {
                options.resolve_tools = Some(Arc::new(move |tool, input, dir, _| {
                    log.lock()
                        .unwrap()
                        .push(format!("{tool} in {}", dir.display()));
                    Box::pin(async move {
                        if tool == "add_markers" {
                            anyhow::bail!("Cut is not an AI timeline.");
                        }
                        Ok(ToolOutput {
                            text: js::stringify(&json!({ "tool": tool, "input": input })),
                            images: (tool == "list_timelines").then(|| vec!["frame.jpg".into()]),
                        })
                    })
                }));
            })
            .scripts([script(move |turn| async move {
                let call = |tool: &'static str, input: Value| {
                    let app = turn.app.clone();
                    async move { app.resolve_tool(tool, input).await }
                };
                let mut results = Vec::new();
                results.push(call("list_timelines", json!({})).await);
                results.push(call("set_current_timeline", json!({ "timeline": "AI - Cut" })).await);
                results.push(call("create_timeline", json!({ "name": "Cut" })).await);
                results.push(
                    call(
                        "add_markers",
                        json!({ "timeline": "Cut", "markers": [{ "frame": 0 }] }),
                    )
                    .await,
                );
                results.push(call("nope", json!({})).await);
                seen.lock().unwrap().extend(results);
                Ok(())
            })])
            .start();
        assert_eq!(
            h.app.resolve_tool("list_timelines", json!({})).await,
            reply(
                "No Linker-Edit turn is running, so Resolve calls are refused.",
                true
            )
        );
        h.send(json!({ "type": "set_approval_mode", "mode": "ask" }));
        h.send(json!({ "type": "send", "text": "build it" }));
        h.client
            .until(|state| !state.meta.approvals.is_empty())
            .await;
        let first = h.state().meta.approvals[0].clone();
        assert_eq!(first.tool, "mcp__linker__create_timeline");
        assert_eq!(first.input, json!({ "name": "Cut" }));
        h.send(json!({ "type": "approve", "id": first.id, "allow": false }));
        h.client
            .until(|state| !state.meta.approvals.is_empty())
            .await;
        let second = h.state().meta.approvals[0].clone();
        h.send(json!({ "type": "approve", "id": second.id, "allow": true }));
        h.turn_done().await;
        assert_eq!(
            *results.lock().unwrap(),
            [
                ToolReply {
                    text: js::stringify(&json!({ "tool": "list_timelines", "input": {} })),
                    images: Some(vec!["frame.jpg".into()]),
                    is_error: false,
                },
                reply(
                    &js::stringify(
                        &json!({ "tool": "set_current_timeline", "input": { "timeline": "AI - Cut" } })
                    ),
                    false
                ),
                reply("The user declined this action.", true),
                reply("Cut is not an AI timeline.", true),
                reply("There is no Resolve tool called nope.", true),
            ]
        );
        let dir = workspace_project().dir.display().to_string();
        assert_eq!(
            *ran.lock().unwrap(),
            [
                format!("list_timelines in {dir}"),
                format!("set_current_timeline in {dir}"),
                format!("add_markers in {dir}")
            ]
        );
    }

    #[tokio::test]
    async fn starts_with_full_access_resolve_changes_run_without_asking_and_the_tools_own_checks_still_refuse()
     {
        let results: Arc<Mutex<Vec<ToolReply>>> = Arc::default();
        let seen = results.clone();
        let h = setup()
            .with(|options| {
                options.resolve_tools = Some(Arc::new(|tool, _, _, _| {
                    Box::pin(async move {
                        if tool == "add_markers" {
                            anyhow::bail!("Cut is not an AI timeline.");
                        }
                        Ok(ToolOutput {
                            text: tool,
                            images: None,
                        })
                    })
                }));
            })
            .scripts([script(move |turn| async move {
                let first = turn
                    .app
                    .resolve_tool("create_timeline", json!({ "name": "Cut" }))
                    .await;
                let second = turn
                    .app
                    .resolve_tool(
                        "add_markers",
                        json!({ "timeline": "Cut", "markers": [{ "frame": 0 }] }),
                    )
                    .await;
                seen.lock().unwrap().extend([first, second]);
                Ok(())
            })])
            .start();
        assert_eq!(h.state().meta.approval_mode, ApprovalMode::Auto);
        h.send(json!({ "type": "send", "text": "build it" }));
        h.turn_done().await;
        assert_eq!(
            *results.lock().unwrap(),
            [
                reply("create_timeline", false),
                reply("Cut is not an AI timeline.", true)
            ]
        );
        assert!(h.state().meta.approvals.is_empty());
    }

    #[tokio::test]
    async fn keeps_supervised_mode_across_restarts() {
        let saved = settings();
        let first = setup().with(with_settings(&saved)).start();
        first.send(json!({ "type": "set_approval_mode", "mode": "ask" }));
        assert_eq!(
            saved.saved(),
            crate::settings::Settings {
                approval_mode: Some(ApprovalMode::Ask),
                ..Default::default()
            }
        );
        first.disconnect();
        let again = setup().with(with_settings(&saved)).start();
        assert_eq!(again.state().meta.approval_mode, ApprovalMode::Ask);
    }

    #[tokio::test]
    async fn starts_new_threads_with_the_agent_picked_last_also_after_a_restart() {
        let saved = settings();
        let first = setup().with(with_settings(&saved)).start();
        assert_eq!(first.state().meta.default_provider, Provider::Claude);
        first.send(json!({ "type": "set_default_provider", "provider": "codex" }));
        assert_eq!(
            saved.saved(),
            crate::settings::Settings {
                default_provider: Some(Provider::Codex),
                ..Default::default()
            }
        );
        // The open thread keeps its own agent; only new threads start with Codex.
        assert_eq!(first.state().meta.provider, Provider::Claude);
        first.send(json!({ "type": "set_default_provider", "provider": "nonsense" }));
        assert_eq!(first.state().meta.default_provider, Provider::Codex);
        first.disconnect();
        let again = setup().with(with_settings(&saved)).start();
        assert_eq!(again.state().meta.default_provider, Provider::Codex);
    }

    #[tokio::test]
    async fn downloads_the_speech_engine_by_itself_but_not_on_a_computer_said_to_have_no_resolve() {
        let saved = settings();
        let prepared = Arc::new(tokio::sync::watch::channel(0usize).0);
        let (resolve, check) = resolve_with(ResolveStatus::SetupNeeded {
            hint: "Install Resolve Studio.".into(),
        });
        let options = |transcriber: Arc<Transcriber>| {
            let (saved, check) = (saved.clone(), check.clone());
            move |options: &mut AppOptions| {
                options.settings = Some(saved);
                options.transcriber = Some(transcriber);
                options.check_resolve = check;
            }
        };
        let started = |count: usize| {
            let mut watch = prepared.subscribe();
            async move {
                let _ = watch.wait_for(|started| *started == count).await;
            }
        };
        let new_transcriber = || {
            Arc::new(Transcriber::new(crate::transcribe::TranscriberOptions {
                dir: "/tmp/transcripts".into(),
                prepare: Arc::new(Counted(prepared.clone())),
                ready: Some(Arc::new(|| false)),
                transcribe_file: None,
            }))
        };

        // A computer that drives an edit rig: the user says so, and the download in progress stops.
        let rig = setup().with(options(new_transcriber())).start();
        rig.app.prepare_transcription();
        started(1).await;
        assert!(rig.state().meta.resolve_here);
        rig.send(json!({ "type": "set_resolve_here", "resolveHere": false }));
        assert_eq!(
            saved.saved(),
            crate::settings::Settings {
                resolve_here: Some(false),
                ..Default::default()
            }
        );
        assert!(!rig.state().meta.resolve_here);
        rig.client
            .until(|state| matches!(state.meta.transcription, TranscriptionStatus::Idle { .. }))
            .await;
        rig.disconnect();

        // After a restart nothing downloads, until Resolve turns up on this computer after all.
        let remote = setup().with(options(new_transcriber())).start();
        remote.app.prepare_transcription();
        remote.app.refresh_resolve().await;
        assert_eq!(*prepared.borrow(), 1);
        assert!(!remote.state().meta.resolve_here);
        *resolve.lock().unwrap() = ResolveStatus::NotRunning;
        remote.app.refresh_resolve().await;
        assert_eq!(
            saved.saved(),
            crate::settings::Settings {
                resolve_here: Some(true),
                ..Default::default()
            }
        );
        assert!(remote.state().meta.resolve_here);
        started(2).await;
    }

    /// Prepare, counting into a shared watch.
    struct Counted(Arc<tokio::sync::watch::Sender<usize>>);

    #[async_trait::async_trait]
    impl PrepareTools for Counted {
        async fn prepare(
            &self,
            _: SetupProgress,
            cancel: CancellationToken,
        ) -> anyhow::Result<WhisperTools> {
            self.0.send_modify(|started| *started += 1);
            cancel.cancelled().await;
            anyhow::bail!("aborted")
        }
    }

    #[tokio::test]
    async fn backs_the_project_up_once_per_turn_before_its_first_resolve_change_and_changes_nothing_when_it_cant()
     {
        let results: Arc<Mutex<Vec<ToolReply>>> = Arc::default();
        let backups: Arc<Mutex<Vec<String>>> = Arc::default();
        let ran: Arc<Mutex<Vec<String>>> = Arc::default();
        let fail = Arc::new(AtomicBool::new(false));
        let (ran_log, backup_log, failing) = (ran.clone(), backups.clone(), fail.clone());
        let calls = |tools: Vec<(&'static str, Value)>| {
            let seen = results.clone();
            script(move |turn| async move {
                for (tool, input) in tools {
                    let reply = turn.app.resolve_tool(tool, input).await;
                    seen.lock().unwrap().push(reply);
                }
                Ok(())
            })
        };
        let h = setup()
            .with(move |options| {
                options.resolve_tools = Some(Arc::new(move |tool, _, _, _| {
                    ran_log.lock().unwrap().push(tool.clone());
                    Box::pin(async move {
                        Ok(ToolOutput {
                            text: tool,
                            images: None,
                        })
                    })
                }));
                options.backup_project = Some(Arc::new(move |dir| {
                    backup_log.lock().unwrap().push(dir.display().to_string());
                    let fail = failing.load(Ordering::SeqCst);
                    Box::pin(async move {
                        if fail {
                            anyhow::bail!("Resolve couldn't export the project.");
                        }
                        Ok("backups/Doc.drp".into())
                    })
                }));
            })
            .scripts([
                calls(vec![
                    ("list_timelines", json!({})),
                    ("create_timeline", json!({ "name": "A" })),
                    ("add_clips", json!({})),
                ]),
                calls(vec![
                    ("create_timeline", json!({ "name": "B" })),
                    ("create_timeline", json!({ "name": "C" })),
                ]),
            ])
            .start();
        h.send(json!({ "type": "send", "text": "one" }));
        h.turn_done().await;
        let dir = workspace_project().dir.display().to_string();
        assert_eq!(*backups.lock().unwrap(), [dir]);
        assert_eq!(
            *ran.lock().unwrap(),
            ["list_timelines", "create_timeline", "add_clips"]
        );
        assert!(notices(&h.state().items).contains(&(
            Tone::Info,
            "Backed up the Resolve project to backups/Doc.drp before changing it.".to_string()
        )));

        fail.store(true, Ordering::SeqCst);
        h.send(json!({ "type": "send", "text": "two" }));
        h.turn_done().await;
        // A failed backup is tried again before the next change, and nothing runs without one.
        assert_eq!(backups.lock().unwrap().len(), 3);
        assert_eq!(ran.lock().unwrap().len(), 3);
        let results = results.lock().unwrap();
        assert_eq!(
            results[3],
            reply(
                "Linker-Edit backs the Resolve project up before every change and couldn't this time, so nothing was changed: Resolve couldn't export the project.",
                true
            )
        );
        assert!(results[4].is_error);
        assert_eq!(results.len(), 5);
    }

    fn applied(timeline_id: &str, previous: &str) -> Applied {
        Applied {
            main: VersionRef {
                id: timeline_id.into(),
                name: "Edit".into(),
            },
            previous: VersionRef {
                id: "t1".into(),
                name: previous.into(),
            },
            was: "AI - Tight".into(),
        }
    }

    #[tokio::test]
    async fn makes_an_ai_timeline_the_main_one_when_the_user_asks_after_a_backup() {
        let order: Arc<Mutex<Vec<String>>> = Arc::default();
        let (backups, applies) = (order.clone(), order.clone());
        let h = setup()
            .with(move |options| {
                options.backup_project = Some(Arc::new(move |_| {
                    backups.lock().unwrap().push("backup".into());
                    Box::pin(async { Ok("backups/Doc.drp".into()) })
                }));
                options.apply_timeline = Some(Arc::new(move |dir, timeline_id| {
                    applies
                        .lock()
                        .unwrap()
                        .push(format!("apply {timeline_id} in {}", dir.display()));
                    Box::pin(async move {
                        Ok(applied(&timeline_id, "Edit (before AI 2026-11-12 09.30)"))
                    })
                }));
            })
            .start();
        h.send(json!({ "type": "apply_timeline", "projectId": "workspace", "timelineId": "t2" }));
        assert_eq!(h.state().meta.applying.as_deref(), Some("workspace"));
        h.client.until(|state| state.meta.applying.is_none()).await;
        let dir = workspace_project().dir.display().to_string();
        assert_eq!(
            *order.lock().unwrap(),
            ["backup".to_string(), format!("apply t2 in {dir}")]
        );
        assert_eq!(
            h.client.toasts(),
            [
                r#""AI - Tight" is now "Edit". The earlier version is kept as "Edit (before AI 2026-11-12 09.30)"."#
            ]
        );
    }

    #[tokio::test]
    async fn doesnt_touch_the_timelines_when_the_backup_fails() {
        let applies: Arc<Mutex<Vec<String>>> = Arc::default();
        let log = applies.clone();
        let h = setup()
            .with(move |options| {
                options.backup_project = Some(Arc::new(|_| {
                    Box::pin(async { anyhow::bail!("Resolve couldn't export the project.") })
                }));
                options.apply_timeline = Some(Arc::new(move |_, timeline_id| {
                    log.lock().unwrap().push(timeline_id.clone());
                    Box::pin(async move { Ok(applied(&timeline_id, "Edit (old)")) })
                }));
            })
            .start();
        h.send(json!({ "type": "apply_timeline", "projectId": "workspace", "timelineId": "t2" }));
        assert_eq!(h.state().meta.applying.as_deref(), Some("workspace"));
        h.client.until(|state| state.meta.applying.is_none()).await;
        assert!(applies.lock().unwrap().is_empty());
        assert_eq!(
            h.client.toasts(),
            ["Couldn't make it the main timeline: Resolve couldn't export the project."]
        );
    }

    #[tokio::test]
    async fn allows_pending_approvals_when_switched_to_auto() {
        let (h, answers) = awaiting_approval().await;
        h.send(json!({ "type": "set_approval_mode", "mode": "yes" }));
        assert!(answers.lock().unwrap().is_empty());
        h.send(json!({ "type": "set_approval_mode", "mode": "auto" }));
        h.turn_done().await;
        assert_eq!(*answers.lock().unwrap(), [true]);
    }

    async fn denies_pending_approvals_on(message: impl Fn(&str) -> Value) {
        let (h, answers) = awaiting_approval().await;
        let thread_id = h.state().meta.thread_id.unwrap();
        h.send(message(&thread_id));
        assert!(h.state().meta.approvals.is_empty());
        h.all_done().await;
        assert_eq!(*answers.lock().unwrap(), [false]);
    }

    #[tokio::test]
    async fn denies_pending_approvals_on_stop() {
        denies_pending_approvals_on(|_| json!({ "type": "stop" })).await;
    }

    #[tokio::test]
    async fn denies_pending_approvals_on_set_provider() {
        denies_pending_approvals_on(|_| json!({ "type": "set_provider", "provider": "codex" }))
            .await;
    }

    #[tokio::test]
    async fn denies_pending_approvals_on_archive_thread() {
        denies_pending_approvals_on(
            |id| json!({ "type": "archive_thread", "threadId": id, "archived": true }),
        )
        .await;
    }

    #[tokio::test]
    async fn denies_pending_approvals_on_delete_thread() {
        denies_pending_approvals_on(|id| json!({ "type": "delete_thread", "threadId": id })).await;
    }
}

fn context(used: u32, expires: Option<&str>) -> Option<ContextUsage> {
    Some(ContextUsage {
        used_tokens: used,
        max_tokens: Some(1_000_000),
        cache_expires_at: expires.map(str::to_string),
    })
}

fn thread_count(store: &MemoryProjectStore, project: &str) -> usize {
    store
        .threads
        .lock()
        .unwrap()
        .get(project)
        .map_or(0, Vec::len)
}

fn saved_threads(store: &MemoryProjectStore, project: &str) -> Vec<SavedThread> {
    store
        .threads
        .lock()
        .unwrap()
        .get(project)
        .cloned()
        .unwrap_or_default()
}

mod chats {
    use super::*;

    #[tokio::test]
    async fn keeps_a_turn_that_outlives_its_deleted_thread_out_of_every_thread() {
        let answers: Answers = Arc::default();
        let seen = answers.clone();
        let h = setup()
            .scripts([script(move |turn| async move {
                let first = turn.sink.request_approval("Write", &json!({})).await;
                seen.lock().unwrap().push(first);
                turn.sink.upsert(assistant("late", "late reply"));
                turn.sink.notice("late notice".into(), Tone::Info);
                let second = turn.sink.request_approval("Write", &json!({})).await;
                seen.lock().unwrap().push(second);
                Ok(())
            })])
            .start();
        h.send(json!({ "type": "send", "text": "go" }));
        h.client
            .until(|state| !state.meta.approvals.is_empty())
            .await;
        let deleted = h.state().meta.thread_id.unwrap();
        h.send(json!({ "type": "delete_thread", "threadId": deleted }));
        h.all_done().await;
        assert_eq!(*answers.lock().unwrap(), [false, false]);
        assert_ne!(h.state().meta.thread_id, Some(deleted));
        assert!(h.state().items.is_empty());
        assert!(h.state().meta.approvals.is_empty());
        assert_eq!(thread_count(&h.store, "workspace"), 0);
    }

    #[tokio::test]
    async fn starts_a_fresh_session_when_the_provider_changes_and_only_then() {
        let h = setup().start();
        h.send(json!({ "type": "set_provider", "provider": "claude" }));
        h.send(json!({ "type": "set_provider", "provider": "codex" }));
        assert_eq!(
            *h.sessions.providers.lock().unwrap(),
            [Provider::Claude, Provider::Codex]
        );
        assert_eq!(h.state().meta.provider, Provider::Codex);
    }

    #[tokio::test]
    async fn keeps_the_chat_going_when_the_provider_changes_giving_the_new_agent_a_summary() {
        let h = setup()
            .scripts([
                script(|turn| async move {
                    turn.sink.upsert(assistant("a", "Built AI - Cut."));
                    turn.sink.context(context(40_000, None));
                    Ok(())
                }),
                quiet(),
            ])
            .start();
        h.send(json!({ "type": "send", "text": "cut it" }));
        h.turn_done().await;
        h.send(json!({ "type": "set_provider", "provider": "codex" }));
        assert_eq!(h.state().meta.context, None);
        assert!(matches!(
            h.state().items.last(),
            Some(ChatItem::Notice { text, .. }) if text.contains("Switched to Codex")
        ));

        h.send(json!({ "type": "send", "text": "shorter" }));
        h.turn_done().await;
        let pattern = regex::Regex::new(
            r"^<earlier_conversation>\n.*\nUser: cut it\nAssistant: Built AI - Cut\.\n</earlier_conversation>\n\nshorter$",
        )
        .unwrap();
        assert!(pattern.is_match(&h.prompts()[1]), "{}", h.prompts()[1]);
        assert_eq!(
            kinds(&h.state().items),
            ["cut it", "assistant", "notice", "shorter"]
        );
    }

    #[tokio::test]
    async fn compacts_on_request_and_before_a_message_when_the_user_asks() {
        let compacted = || {
            script(|turn| async move {
                turn.sink.notice("Compacted.".into(), Tone::Info);
                turn.sink.context(context(9_000, None));
                Ok(())
            })
        };
        let h = setup()
            .scripts([quiet(), compacted(), compacted(), quiet()])
            .start();
        // Nothing to compact before the first turn.
        h.send(json!({ "type": "compact" }));
        assert!(!h.state().meta.busy);

        h.send(json!({ "type": "send", "text": "one" }));
        h.turn_done().await;
        h.send(json!({ "type": "compact" }));
        h.turn_done().await;
        assert_eq!(h.state().meta.context.unwrap().used_tokens, 9_000);
        h.send(json!({ "type": "send", "text": "two", "compactFirst": true }));
        h.turn_done().await;
        assert_eq!(h.prompts(), ["one", "/compact", "/compact", "two"]);
        assert_eq!(kinds(&h.state().items), ["one", "notice", "two", "notice"]);
    }

    #[tokio::test]
    async fn gives_a_client_that_connects_mid_turn_the_same_state() {
        let gate = Gate::default();
        let opened = gate.clone();
        let h = setup()
            .scripts([script(move |turn| async move {
                turn.sink.upsert(assistant("a1", "partial"));
                opened.opened().await;
                Ok(())
            })])
            .start();
        h.send(json!({ "type": "send", "text": "go" }));
        let (late, _) = h.connect();
        assert_eq!(late.state(), h.state());
        gate.open();
        h.turn_done().await;
        late.until(|state| !state.meta.busy).await;
        assert_eq!(late.state(), h.state());
    }
}

mod restarts {
    use super::*;

    fn workspace_store(threads: Vec<SavedThread>) -> Arc<MemoryProjectStore> {
        Arc::new(memory_project_store(vec![LoadedProject {
            project: workspace_project(),
            threads,
        }]))
    }

    #[tokio::test]
    async fn keeps_the_thread_and_continues_the_providers_conversation() {
        let store = workspace_store(Vec::new());
        let first = setup()
            .store(store.clone())
            .scripts([script(|turn| async move {
                turn.sink.upsert(assistant("a", "Built AI - Cut."));
                turn.sink
                    .context(context(40_000, Some("2026-11-08T12:00:00.000Z")));
                Ok(())
            })])
            .start();
        first.send(json!({ "type": "set_provider", "provider": "claude" }));
        first.send(json!({ "type": "send", "text": "cut it" }));
        first.turn_done().await;

        let second = setup().store(store.clone()).scripts([quiet()]).start();
        let mut expected = first.state();
        expected.meta.resolve = ResolveStatus::Checking;
        assert_eq!(second.state(), expected);
        assert_eq!(
            *second.sessions.resume_ids.lock().unwrap(),
            [Some("claude-1".to_string())]
        );
        assert_eq!(*second.sessions.dirs.lock().unwrap(), ["/work"]);
        second.send(json!({ "type": "send", "text": "shorter" }));
        second.turn_done().await;
        assert_eq!(second.prompts(), ["shorter"]);

        // A new thread starts empty and isn't saved until it has a message; asking again reuses it.
        second.send(json!({ "type": "new_thread", "projectId": "workspace" }));
        let fresh = second.state().meta.thread_id;
        assert!(second.state().items.is_empty());
        assert_eq!(second.state().meta.context, None);
        second.send(json!({ "type": "new_thread", "projectId": "workspace" }));
        assert_eq!(second.state().meta.thread_id, fresh);
        let saved = saved_threads(&store, "workspace");
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].title, "cut it");
        assert_eq!(saved[0].resume_id.as_deref(), Some("claude-1"));
    }

    #[tokio::test]
    async fn saves_the_users_message_when_the_turn_starts_so_a_crash_mid_turn_doesnt_lose_it() {
        let gate = Gate::default();
        let opened = gate.clone();
        let h = setup()
            .scripts([script(move |_| async move {
                opened.opened().await;
                Ok(())
            })])
            .start();
        h.send(json!({ "type": "send", "text": "cut the silences" }));
        h.client.until(|state| !state.items.is_empty()).await;
        let saved = saved_threads(&h.store, "workspace");
        assert!(matches!(
            saved[0].items.as_slice(),
            [ChatItem::User { text, .. }] if text == "cut the silences"
        ));
        gate.open();
        h.turn_done().await;
    }

    #[tokio::test]
    async fn starts_a_new_conversation_with_a_summary_when_the_earlier_one_cant_be_continued() {
        let store = workspace_store(vec![SavedThread {
            resume_id: Some("gone".into()),
            items: vec![ChatItem::User {
                id: "1".into(),
                text: "cut it".into(),
            }],
            ..saved_thread()
        }]);
        let h = setup()
            .store(store.clone())
            .scripts([
                script(|_| async { anyhow::bail!("No conversation found") }),
                script(|turn| async move {
                    turn.sink.upsert(assistant("a", "Done."));
                    Ok(())
                }),
            ])
            .start();
        h.send(json!({ "type": "send", "text": "shorter" }));
        h.turn_done().await;
        assert_eq!(
            *h.sessions.resume_ids.lock().unwrap(),
            [Some("gone".to_string()), None]
        );
        let prompts = h.prompts();
        assert_eq!(prompts[0], "shorter");
        assert!(
            regex::Regex::new(r"User: cut it\n</earlier_conversation>\n\nshorter$")
                .unwrap()
                .is_match(&prompts[1])
        );
        let notices = notices(&h.state().items);
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].0, Tone::Info);
        assert!(
            notices[0]
                .1
                .contains("Couldn't continue the earlier conversation (No conversation found)")
        );
        let saved = saved_threads(&store, "workspace");
        assert_eq!(saved[0].resume_id.as_deref(), Some("claude-2"));
    }
}

fn earlier_cut(commit: String) -> HistoryEntry {
    HistoryEntry {
        commit,
        date: "2026-11-07T10:00:00+01:00".into(),
        author: "Claude".into(),
        agent: Some(Provider::Claude),
        subject: "Earlier cut".into(),
        prompt: Some("Earlier cut".into()),
        actions: Vec::new(),
        changes: Vec::new(),
        thread: None,
        timelines: Vec::new(),
    }
}

mod edit_history {
    use super::*;

    fn with_history(scripts: Vec<Script>) -> (Harness, Arc<FakeHistory>) {
        let history = Arc::new(FakeHistory {
            changes: vec![r#""AI - Cut": new timeline, 3 clips, 0:10"#.into()],
            recent: "9f8e7d6 2026-11-07 10:00 Claude: Earlier cut".into(),
            log: Some(Box::new(|count| vec![earlier_cut(format!("c{count}"))])),
            ..FakeHistory::default()
        });
        let h = setup()
            .history(history.clone())
            .with(|options| {
                options.check_resolve =
                    Arc::new(|| Box::pin(async { connected(Some("Doc"), Some("doc-id")) }));
            })
            .scripts(scripts)
            .start();
        (h, history)
    }

    #[tokio::test]
    async fn saves_before_every_turn_and_after_a_turn_that_changed_resolve_with_what_the_agent_did()
    {
        let (h, history) = with_history(vec![
            script(|turn| async move {
                let sink = &turn.sink;
                sink.upsert(tool(
                    "w",
                    "mcp__linker__create_timeline",
                    json!({ "name": "Cut" }),
                    ToolStatus::Running,
                ));
                sink.upsert(tool(
                    "r",
                    "mcp__linker__list_timelines",
                    json!({}),
                    ToolStatus::Done,
                ));
                sink.upsert(status("w", ToolStatus::Done));
                sink.upsert(assistant("a", "Built AI - Cut."));
                Ok(())
            }),
            script(|turn| async move {
                turn.sink.upsert(tool(
                    "r2",
                    "mcp__linker__list_timelines",
                    json!({}),
                    ToolStatus::Done,
                ));
                Ok(())
            }),
        ]);
        h.app.refresh_resolve().await;
        h.send(json!({ "type": "send", "text": "cut it" }));
        h.turn_done().await;
        h.send(json!({ "type": "send", "text": "what is on it?" }));
        h.turn_done().await;

        assert_eq!(
            history.records(),
            [
                None,
                Some(AgentTurn {
                    provider: Provider::Claude,
                    prompt: "cut it".into(),
                    actions: vec![r#"create_timeline {"name":"Cut"}"#.into()],
                    summary: "Built AI - Cut.".into(),
                    thread: h.state().meta.thread_id,
                }),
                None,
            ]
        );
        let prompts = h.prompts();
        assert!(
            regex::Regex::new(
                r"^<edit_history>\n.*\n9f8e7d6 2026-11-07 10:00 Claude: Earlier cut\n</edit_history>\n\ncut it$"
            )
            .unwrap()
            .is_match(&prompts[0]),
            "{}",
            prompts[0]
        );
        assert_eq!(prompts[1], "what is on it?");
        // The project had no Resolve project yet: its first turn links it to the one open.
        assert_eq!(
            notices(&h.state().items),
            [
                (
                    Tone::Info,
                    r#"Linked this project to the Resolve project "Doc" in the local database "Local Database"."#
                        .to_string()
                ),
                (
                    Tone::Info,
                    r#"Saved to edit history (abc1234): "AI - Cut": new timeline, 3 clips, 0:10"#
                        .to_string()
                ),
            ]
        );
        let saved = h.store.project("workspace").unwrap();
        assert_eq!(
            saved.link,
            Some(ResolveLink {
                kind: ResolveLinkKind::Local,
                name: "Doc".into(),
                project_id: Some("doc-id".into())
            })
        );
        assert_eq!(
            saved
                .local
                .database
                .map(|database| (database.r#type, database.name)),
            Some(("Disk".to_string(), "Local Database".to_string()))
        );
        assert_eq!(history.commits(), [r#"Link to Resolve project "Doc""#]);
        let project = &h.state().meta.projects[0];
        assert_eq!((project.name.as_str(), project.open), ("Doc", true));
    }

    #[tokio::test]
    async fn sends_the_history_on_request_and_to_every_client_after_each_save() {
        let (h, _) = with_history(vec![script(|turn| async move {
            turn.sink.upsert(tool(
                "w",
                "mcp__linker__create_timeline",
                json!({}),
                ToolStatus::Done,
            ));
            Ok(())
        })]);
        h.send(json!({ "type": "load_history" }));
        h.app.refresh_resolve().await;
        h.send(json!({ "type": "send", "text": "cut it" }));
        h.turn_done().await;
        h.client
            .until_checked(|_| h.client.histories.lock().unwrap().len() == 2)
            .await;
        let commits: Vec<Vec<String>> = h
            .client
            .histories
            .lock()
            .unwrap()
            .iter()
            .map(|entries| entries.iter().map(|entry| entry.commit.clone()).collect())
            .collect();
        assert_eq!(commits, [vec!["c0".to_string()], vec!["c2".to_string()]]);
    }

    async fn still_saves_a_turns_resolve_changes_after(message: impl Fn(&str) -> Value) {
        let gate = Gate::default();
        let opened = gate.clone();
        let (h, history) = with_history(vec![script(move |turn| async move {
            turn.sink.upsert(tool(
                "w",
                "mcp__linker__create_timeline",
                json!({}),
                ToolStatus::Running,
            ));
            opened.opened().await;
            Ok(())
        })]);
        h.app.refresh_resolve().await;
        h.send(json!({ "type": "send", "text": "cut it" }));
        h.client
            .until(|state| state.items.iter().any(|item| item_id(item) == "w"))
            .await;
        h.send(message(&h.state().meta.thread_id.unwrap()));
        gate.open();
        h.all_done().await;
        let last = history.records().last().cloned().flatten().unwrap();
        assert_eq!(last.prompt, "cut it");
        assert_eq!(last.actions, ["create_timeline"]);
    }

    #[tokio::test]
    async fn still_saves_a_turns_resolve_changes_after_its_thread_was_deleted() {
        still_saves_a_turns_resolve_changes_after(
            |id| json!({ "type": "delete_thread", "threadId": id }),
        )
        .await;
    }

    #[tokio::test]
    async fn still_saves_a_turns_resolve_changes_after_its_project_was_removed() {
        still_saves_a_turns_resolve_changes_after(
            |_| json!({ "type": "remove_project", "projectId": "workspace" }),
        )
        .await;
    }
}

/// A renderer the test drives: each render waits for events the test emits.
#[derive(Default)]
struct FakeRenderer {
    renders: Mutex<Vec<FakeRender>>,
}

struct FakeRender {
    timeline_id: Option<String>,
    emit: OnPreviewEvent,
    stopped: Arc<AtomicBool>,
}

impl FakeRenderer {
    fn emit(&self, index: usize, event: PreviewEvent) {
        let emit = self.renders.lock().unwrap()[index].emit.clone();
        emit(event);
    }

    fn count(&self) -> usize {
        self.renders.lock().unwrap().len()
    }
}

impl PreviewRenderer for FakeRenderer {
    fn render(&self, timeline_id: Option<String>, on_event: OnPreviewEvent) -> PreviewJob {
        let stop = CancellationToken::new();
        let stopped = Arc::new(AtomicBool::new(false));
        self.renders.lock().unwrap().push(FakeRender {
            timeline_id,
            emit: on_event.clone(),
            stopped: stopped.clone(),
        });
        let watched = stop.clone();
        tokio::spawn(async move {
            watched.cancelled().await;
            stopped.store(true, Ordering::SeqCst);
            on_event(PreviewEvent::Error {
                message: "Preview stopped.".into(),
                stopped: Some(true),
            });
        });
        PreviewJob::new(stop)
    }

    fn file(&self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
}

fn rendering(timeline: Option<(&str, &str)>, percent: f64) -> PreviewState {
    PreviewState::Rendering {
        timeline: timeline.map(|(id, name)| linker_edit_protocol::PreviewTimeline {
            id: id.into(),
            name: name.into(),
        }),
        percent,
    }
}

mod preview {
    use super::*;

    #[tokio::test]
    async fn renders_a_timeline_reports_progress_and_serves_the_result_outdated_once_resolve_is_saved_again()
     {
        let renderer = Arc::new(FakeRenderer::default());
        let preview = renderer.clone();
        let h = setup()
            .history(Arc::new(FakeHistory::default()))
            .with(move |options| {
                options.preview = Some(preview);
                options.check_resolve =
                    Arc::new(|| Box::pin(async { connected(Some("Doc"), None) }));
            })
            .scripts([script(|turn| async move {
                turn.sink.upsert(tool(
                    "w",
                    "mcp__linker__create_timeline",
                    json!({}),
                    ToolStatus::Done,
                ));
                Ok(())
            })])
            .start();
        h.app.refresh_resolve().await;
        h.send(json!({ "type": "render_preview", "timelineId": "t1" }));
        assert_eq!(h.state().meta.preview, rendering(None, 0.0));

        // Resolve does one thing at a time: no turn while a render runs, and no second render.
        h.send(json!({ "type": "send", "text": "cut it" }));
        h.send(json!({ "type": "render_preview" }));
        assert!(h.state().items.is_empty());
        assert_eq!(renderer.count(), 1);

        renderer.emit(
            0,
            PreviewEvent::Start {
                timeline: VersionRef {
                    id: "t1".into(),
                    name: "AI - Cut".into(),
                },
            },
        );
        renderer.emit(0, PreviewEvent::Progress { percent: 40.0 });
        assert_eq!(
            h.state().meta.preview,
            rendering(Some(("t1", "AI - Cut")), 40.0)
        );
        renderer.emit(
            0,
            PreviewEvent::Done {
                file: "/previews/preview-1.mp4".into(),
            },
        );
        assert!(matches!(
            h.state().meta.preview,
            PreviewState::Ready { url, outdated: false, .. } if url == "/preview/preview-1.mp4"
        ));

        h.send(json!({ "type": "send", "text": "cut it" }));
        h.turn_done().await;
        assert!(matches!(
            h.state().meta.preview,
            PreviewState::Ready { outdated: true, .. }
        ));
    }

    #[tokio::test]
    async fn stops_a_render_and_shows_failures() {
        let renderer = Arc::new(FakeRenderer::default());
        let preview = renderer.clone();
        let h = setup()
            .with(move |options| options.preview = Some(preview))
            .start();
        h.send(json!({ "type": "render_preview" }));
        h.send(json!({ "type": "stop_preview" }));
        // The stand-in stops in a task of its own, as Resolve's renderer does.
        h.client
            .until(|state| state.meta.preview == PreviewState::Idle)
            .await;
        {
            let renders = renderer.renders.lock().unwrap();
            assert_eq!(renders[0].timeline_id, None);
            assert!(renders[0].stopped.load(Ordering::SeqCst));
        }

        h.send(json!({ "type": "render_preview" }));
        renderer.emit(
            1,
            PreviewEvent::Error {
                message: "Resolve has no timeline open.".into(),
                stopped: None,
            },
        );
        assert_eq!(
            h.state().meta.preview,
            PreviewState::Error {
                message: "Resolve has no timeline open.".into()
            }
        );
    }
}

fn linked(name: &str, project_id: Option<&str>) -> crate::projects::store::SavedProject {
    crate::projects::store::SavedProject {
        id: name.to_lowercase(),
        dir: format!("/projects/{name}").into(),
        name: None,
        link: Some(ResolveLink {
            kind: ResolveLinkKind::Local,
            name: name.into(),
            project_id: project_id.map(str::to_string),
        }),
        local: Default::default(),
        icon: None,
        pinned: false,
        archived: false,
        collection: None,
        setup: false,
    }
}

fn store_of(projects: Vec<crate::projects::store::SavedProject>) -> Arc<MemoryProjectStore> {
    Arc::new(memory_project_store(
        projects
            .into_iter()
            .map(|project| LoadedProject {
                project,
                threads: Vec::new(),
            })
            .collect(),
    ))
}

/// A render that never ends by itself.
struct EndlessRenderer(Arc<Mutex<Vec<Option<String>>>>);

impl PreviewRenderer for EndlessRenderer {
    fn render(&self, timeline_id: Option<String>, _: OnPreviewEvent) -> PreviewJob {
        self.0.lock().unwrap().push(timeline_id);
        PreviewJob::new(CancellationToken::new())
    }

    fn file(&self, _: &str) -> Option<std::path::PathBuf> {
        None
    }
}

mod projects {
    use super::*;

    #[tokio::test]
    async fn names_project_md_with_the_systems_separators_as_node_did() {
        let h = setup().start();
        h.send(json!({ "type": "load_project_file" }));
        let file = h.client.files.lock().unwrap().last().cloned().unwrap().1;
        // The workspace project is in "/work"; on Windows that is \work, as Node's path.join wrote it.
        let expected = if cfg!(windows) {
            "\\work\\PROJECT.md"
        } else {
            "/work/PROJECT.md"
        };
        assert_eq!(file, expected);
    }

    #[tokio::test]
    async fn shows_a_preview_only_in_its_own_projects_threads() {
        let h = setup()
            .store(store_of(vec![
                linked("Interview", None),
                linked("Promo", None),
            ]))
            .with(|options| options.preview = Some(Arc::new(EndlessRenderer(Arc::default()))))
            .start();
        assert_eq!(h.state().meta.project_id.as_deref(), Some("interview"));
        h.send(json!({ "type": "render_preview" }));
        assert!(matches!(
            h.state().meta.preview,
            PreviewState::Rendering { .. }
        ));
        h.send(json!({ "type": "new_thread", "projectId": "promo" }));
        assert_eq!(h.state().meta.preview, PreviewState::Idle);
        assert!(
            h.state()
                .meta
                .blocked
                .unwrap()
                .message
                .contains("rendering a preview")
        );
    }

    #[tokio::test]
    async fn doesnt_preview_another_resolve_project_as_this_projects() {
        let renders: Arc<Mutex<Vec<Option<String>>>> = Arc::default();
        let preview = Arc::new(EndlessRenderer(renders.clone()));
        let timeline = PlayableTimeline {
            id: "t1".into(),
            name: "Edit".into(),
            commit: None,
            fps: 25.0,
            width: 1920,
            height: 1080,
            duration: 10.0,
            video: Vec::new(),
            audio: Vec::new(),
            markers: Vec::new(),
            replaces: None,
        };
        let h = setup()
            .store(store_of(vec![linked("Interview", Some("int-id"))]))
            .history(Arc::new(FakeHistory {
                timeline: Some(timeline.clone()),
                ..FakeHistory::default()
            }))
            .with(move |options| {
                options.preview = Some(preview);
                options.check_resolve =
                    Arc::new(|| Box::pin(async { connected(Some("Promo"), Some("promo-id")) }));
            })
            .start();
        h.app.refresh_resolve().await;
        h.send(json!({ "type": "render_preview", "timelineId": "t1" }));
        assert!(renders.lock().unwrap().is_empty());
        let toasts = h.client.toasts();
        assert_eq!(toasts.len(), 1);
        assert!(toasts[0].contains(r#"Resolve has "Promo" open"#));
        let live = h.app.playable_timeline(Some("interview"), None, None).await;
        assert!(
            live.unwrap_err()
                .to_string()
                .contains(r#"Resolve has "Promo" open"#)
        );
        // A saved version comes from the edit history, not from Resolve.
        let saved = h
            .app
            .playable_timeline(Some("interview"), Some("t1".into()), Some("abc1234".into()))
            .await
            .unwrap();
        assert_eq!(saved, Some(timeline));
    }

    #[tokio::test]
    async fn refuses_a_turn_while_resolve_has_another_project_open_and_runs_it_once_the_project_is_open()
     {
        let (resolve, check) = resolve_with(connected(Some("Promo"), Some("promo-id")));
        let h = setup()
            .store(store_of(vec![linked("Interview", Some("int-id"))]))
            .with(move |options| options.check_resolve = check)
            .scripts([quiet()])
            .start();
        h.app.refresh_resolve().await;
        let blocked = h.state().meta.blocked.unwrap();
        assert_eq!(
            blocked.message,
            r#"Resolve has "Promo" open, not this project's "Interview". Open "Interview" in Resolve to continue."#
        );
        assert!(!blocked.can_open);
        let project = &h.state().meta.projects[0];
        assert_eq!((project.name.as_str(), project.open), ("Interview", false));

        h.send(json!({ "type": "send", "text": "cut it" }));
        h.client
            .until_checked(|_| !h.client.refused().is_empty())
            .await;
        assert_eq!(h.client.refused(), ["cut it"]);
        assert_eq!(h.client.toasts(), [blocked.message]);
        assert!(h.prompts().is_empty());
        assert!(h.state().items.is_empty());

        *resolve.lock().unwrap() = connected(Some("Interview"), Some("int-id"));
        h.send(json!({ "type": "send", "text": "cut it" }));
        h.client.until(|state| !state.items.is_empty()).await;
        h.turn_done().await;
        assert_eq!(h.prompts(), ["cut it"]);
        assert_eq!(h.state().meta.blocked, None);
    }

    #[tokio::test]
    async fn wont_link_a_new_project_to_a_resolve_project_another_project_has() {
        let store = store_of(vec![linked("Doc", Some("doc-id")), workspace_project()]);
        let h = setup()
            .store(store.clone())
            .with(|options| {
                options.check_resolve =
                    Arc::new(|| Box::pin(async { connected(Some("Doc"), Some("doc-id")) }));
            })
            .start();
        h.app.refresh_resolve().await;
        assert_eq!(h.state().meta.project_id.as_deref(), Some("doc"));
        h.send(json!({ "type": "new_thread", "projectId": "workspace" }));
        assert_eq!(
            h.state().meta.blocked.unwrap().message,
            r#"Resolve has "Doc" open, which belongs to the project "Doc". Switch to that project, or open another one in Resolve."#
        );
        h.send(json!({ "type": "send", "text": "cut it" }));
        h.client
            .until_checked(|_| !h.client.refused().is_empty())
            .await;
        assert_eq!(store.project("workspace").unwrap().link, None);
    }

    #[tokio::test]
    async fn lets_one_thread_work_in_resolve_at_a_time() {
        let gate = Gate::default();
        let opened = gate.clone();
        let h = setup()
            .scripts([
                script(move |_| async move {
                    opened.opened().await;
                    Ok(())
                }),
                quiet(),
            ])
            .start();
        h.send(json!({ "type": "send", "text": "first" }));
        h.client.until(|state| !state.items.is_empty()).await;
        h.send(json!({ "type": "new_thread", "projectId": "workspace" }));
        assert_eq!(
            h.state().meta.blocked,
            Some(linker_edit_protocol::Blocked {
                message: r#"Claude is working in "first". One thread works in Resolve at a time."#
                    .into(),
                can_open: false
            })
        );
        h.send(json!({ "type": "send", "text": "second" }));
        assert_eq!(h.client.refused(), ["second"]);
        gate.open();
        h.all_done().await;
        h.send(json!({ "type": "send", "text": "second" }));
        h.all_done().await;
        assert_eq!(h.prompts(), ["first", "second"]);
    }

    #[tokio::test]
    async fn learns_the_projects_id_when_it_is_open_and_follows_a_rename_in_resolve() {
        let (resolve, check) = resolve_with(connected(Some("Doc"), Some("doc-id")));
        let history = Arc::new(FakeHistory::default());
        let store = store_of(vec![linked("Doc", None)]);
        let h = setup()
            .store(store.clone())
            .history(history.clone())
            .with(move |options| options.check_resolve = check)
            .start();
        h.app.refresh_resolve().await;
        let link = |name: &str| ResolveLink {
            kind: ResolveLinkKind::Local,
            name: name.into(),
            project_id: Some("doc-id".into()),
        };
        assert_eq!(store.project("doc").unwrap().link, Some(link("Doc")));

        *resolve.lock().unwrap() = connected(Some("Doc v2"), Some("doc-id"));
        h.app.refresh_resolve().await;
        assert_eq!(store.project("doc").unwrap().link, Some(link("Doc v2")));
        assert_eq!(
            history.commits(),
            [
                r#"Link to Resolve project "Doc""#,
                r#"Resolve project renamed to "Doc v2""#
            ]
        );
        let project = &h.state().meta.projects[0];
        assert_eq!((project.name.as_str(), project.open), ("Doc v2", true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn asks_resolve_when_a_page_says_to() {
        let store = store_of(vec![linked("Doc", None)]);
        let h = setup()
            .store(store)
            .with(|options| {
                options.check_resolve =
                    Arc::new(|| Box::pin(async { connected(Some("Doc"), Some("doc-id")) }));
            })
            .start();
        // A message is handled with the app locked; refresh_resolve used to lock it again, and hang the server.
        let (app, id, runtime) = (h.app.clone(), h.id, tokio::runtime::Handle::current());
        let (done, handled) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _runtime = runtime.enter();
            app.handle_message(id, message(json!({ "type": "refresh_resolve" })).unwrap());
            let _ = done.send(());
        });
        handled
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("refresh_resolve was handled");
        h.app.refresh_resolve().await;
        assert!(h.state().meta.projects[0].open);
    }

    #[tokio::test]
    async fn adds_projects_from_resolve_its_database_and_blackmagic_cloud_once_each() {
        let store = store_of(vec![workspace_project()]);
        let h = setup()
            .store(store.clone())
            .with(|options| {
                options.check_resolve =
                    Arc::new(|| Box::pin(async { connected(Some("Promo"), Some("promo-id")) }));
            })
            .start();
        h.app.refresh_resolve().await;

        h.send(json!({ "type": "add_project", "source": { "kind": "open" } }));
        assert_eq!(h.state().meta.project_id.as_deref(), Some("project-1"));
        let promo = h.state().meta.projects[1].clone();
        assert_eq!(promo.name, "Promo");
        assert_eq!(
            promo.link,
            Some(ResolveLink {
                kind: ResolveLinkKind::Local,
                name: "Promo".into(),
                project_id: Some("promo-id".into())
            })
        );
        assert_eq!(
            promo.local.database.map(|database| database.name),
            Some("Local Database".into())
        );
        assert!(promo.open);
        h.send(json!({ "type": "add_project", "source": { "kind": "open" } }));
        assert_eq!(h.client.toasts(), [r#""Promo" is already in Linker-Edit."#]);

        h.send(json!({ "type": "add_project", "source": { "kind": "cloud", "name": " " } }));
        assert_eq!(
            h.client.toasts().last().unwrap(),
            "A cloud project needs its name."
        );
        h.send(json!({ "type": "add_project", "source": { "kind": "cloud", "name": "Client X" } }));
        h.send(json!({
            "type": "add_project",
            "source": {
                "kind": "library",
                "name": "Old",
                "folder": ["2025"],
                "database": { "type": "PostgreSQL", "name": "Team", "host": "10.0.0.5" },
            },
        }));
        let projects = h.state().meta.projects;
        assert_eq!(projects[2].name, "Client X");
        let cloud = projects[2].link.clone().unwrap();
        assert_eq!(
            (cloud.kind, cloud.project_id),
            (ResolveLinkKind::Cloud, None)
        );
        assert_eq!(projects[2].local, Default::default());
        assert_eq!(projects[3].name, "Old");
        assert_eq!(
            projects[3].link.as_ref().unwrap().kind,
            ResolveLinkKind::Network
        );
        assert_eq!(projects[3].local.folder, Some(vec!["2025".to_string()]));
        assert_eq!(
            projects[3].local.database.as_ref().unwrap().name,
            "Team".to_string()
        );
        assert_eq!(store.projects.lock().unwrap().len(), 4);
    }

    struct Projects {
        resolve: Arc<Mutex<ResolveStatus>>,
        opens: Mutex<Vec<String>>,
        fail: AtomicBool,
    }

    #[async_trait::async_trait]
    impl ResolveProjects for Projects {
        async fn library(&self) -> anyhow::Result<linker_edit_protocol::ResolveLibrary> {
            Ok(linker_edit_protocol::ResolveLibrary {
                database: None,
                databases: Vec::new(),
                projects: Vec::new(),
            })
        }

        async fn open(
            &self,
            link: &ResolveLink,
            _: &linker_edit_protocol::LocalLink,
        ) -> anyhow::Result<OpenedProject> {
            self.opens.lock().unwrap().push(link.name.clone());
            if self.fail.load(Ordering::SeqCst) {
                anyhow::bail!("Resolve couldn't find or open the project.");
            }
            *self.resolve.lock().unwrap() = connected(Some("Interview"), Some("int-id"));
            Ok(OpenedProject {
                project: OpenedProjectName {
                    name: "Interview".into(),
                    id: Some("int-id".into()),
                },
                database: Some(linker_edit_protocol::ResolveDatabase {
                    r#type: "Disk".into(),
                    name: "Local Database".into(),
                    host: None,
                }),
                folder: Some(vec!["Clients".into()]),
            })
        }
    }

    #[tokio::test]
    async fn opens_a_project_in_resolve_and_says_when_it_couldnt() {
        let (resolve, check) = resolve_with(connected(Some("Promo"), Some("promo-id")));
        let projects = Arc::new(Projects {
            resolve,
            opens: Mutex::default(),
            fail: AtomicBool::new(false),
        });
        let store = store_of(vec![linked("Interview", None)]);
        let opener = projects.clone();
        let h = setup()
            .store(store.clone())
            .with(move |options| {
                options.check_resolve = check;
                options.resolve_projects = Some(opener);
            })
            .start();
        h.app.refresh_resolve().await;
        assert!(h.state().meta.blocked.unwrap().can_open);

        projects.fail.store(true, Ordering::SeqCst);
        h.send(json!({ "type": "open_in_resolve", "projectId": "interview" }));
        assert_eq!(h.state().meta.opening.as_deref(), Some("interview"));
        h.client.until(|state| state.meta.opening.is_none()).await;
        assert_eq!(
            h.client.toasts(),
            [r#"Couldn't open "Interview" in Resolve: Resolve couldn't find or open the project."#]
        );

        projects.fail.store(false, Ordering::SeqCst);
        h.send(json!({ "type": "open_in_resolve", "projectId": "interview" }));
        h.client
            .until(|state| state.meta.opening.is_none() && state.meta.blocked.is_none())
            .await;
        assert_eq!(*projects.opens.lock().unwrap(), ["Interview", "Interview"]);
        let saved = store.project("interview").unwrap();
        assert_eq!(
            saved.link.and_then(|link| link.project_id).as_deref(),
            Some("int-id")
        );
        assert_eq!(saved.local.folder, Some(vec!["Clients".to_string()]));
        assert_eq!(
            saved
                .local
                .database
                .map(|database| database.name)
                .as_deref(),
            Some("Local Database")
        );
    }

    #[tokio::test]
    async fn renames_archives_unarchives_and_deletes_threads() {
        let store = Arc::new(memory_project_store(vec![LoadedProject {
            project: workspace_project(),
            threads: vec![SavedThread {
                items: vec![ChatItem::User {
                    id: "1".into(),
                    text: "cut it".into(),
                }],
                ..saved_thread()
            }],
        }]));
        let h = setup().store(store.clone()).start();
        assert_eq!(h.state().meta.thread_id.as_deref(), Some("thread-1"));
        let summary = || {
            h.state().meta.projects[0]
                .threads
                .iter()
                .find(|thread| thread.id == "thread-1")
                .cloned()
        };

        h.send(
            json!({ "type": "rename_thread", "threadId": "thread-1", "title": "  Rough cut  " }),
        );
        assert_eq!(summary().unwrap().title, "Rough cut");
        assert!(!summary().unwrap().archived);
        h.send(json!({ "type": "archive_thread", "threadId": "thread-1", "archived": true }));
        assert!(summary().unwrap().archived);
        assert_ne!(h.state().meta.thread_id.as_deref(), Some("thread-1"));
        let saved = saved_threads(&store, "workspace");
        assert_eq!(saved[0].title, "Rough cut");
        assert!(saved[0].archived_at.is_some());

        h.send(json!({ "type": "archive_thread", "threadId": "thread-1", "archived": false }));
        assert!(!summary().unwrap().archived);
        // Archiving a thread with nothing in it just drops it.
        let empty = h.state().meta.thread_id.unwrap();
        h.send(json!({ "type": "archive_thread", "threadId": empty, "archived": true }));
        assert!(
            !h.state().meta.projects[0]
                .threads
                .iter()
                .any(|thread| thread.id == empty)
        );

        h.send(json!({ "type": "delete_thread", "threadId": "thread-1" }));
        assert_eq!(summary(), None);
        assert_eq!(thread_count(&store, "workspace"), 0);
    }

    #[tokio::test]
    async fn removes_a_project_from_linker_edit_only_moving_its_viewers_to_another_project() {
        let store = store_of(vec![linked("Doc", Some("doc-id")), workspace_project()]);
        let h = setup().store(store.clone()).start();
        assert_eq!(h.state().meta.project_id.as_deref(), Some("doc"));
        h.send(json!({ "type": "remove_project", "projectId": "doc" }));
        let ids: Vec<String> = h
            .state()
            .meta
            .projects
            .iter()
            .map(|project| project.id.clone())
            .collect();
        assert_eq!(ids, ["workspace"]);
        assert_eq!(h.state().meta.project_id.as_deref(), Some("workspace"));
        let kept: Vec<String> = store
            .projects
            .lock()
            .unwrap()
            .iter()
            .map(|project| project.id.clone())
            .collect();
        assert_eq!(kept, ["workspace"]);
    }
}

mod sidebar {
    use super::*;

    fn ms(text: &Option<String>) -> i64 {
        crate::thread::parse_ms(text.as_deref().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn pins_settles_snoozes_and_reads_threads_and_keeps_it_all_across_a_restart() {
        let store = store_of(vec![workspace_project()]);
        let h = setup()
            .store(store.clone())
            .scripts([script(|turn| async move {
                turn.sink.upsert(assistant("a", "Cut."));
                Ok(())
            })])
            .start();
        h.send(json!({ "type": "send", "text": "cut it" }));
        h.turn_done().await;
        let thread_id = h.state().meta.thread_id.unwrap();
        let summary = || {
            h.state().meta.projects[0]
                .threads
                .iter()
                .find(|thread| thread.id == thread_id)
                .cloned()
                .unwrap()
                .sidebar
        };
        let now = summary();
        assert_eq!(
            (now.pinned_at, now.settled_at, now.last_read_at),
            (None, None, None)
        );
        assert!(now.last_sent_at.is_some());
        assert!(now.turn_ended_at.is_some());

        h.send(json!({ "type": "pin_thread", "threadId": thread_id, "pinned": true }));
        assert!(summary().pinned_at.is_some());

        // Settling unpins and wakes it, as in T3 Code.
        let hour = js::iso_from_ms(js::now_millis() + 3_600_000);
        h.send(json!({ "type": "snooze_thread", "threadId": thread_id, "until": hour }));
        assert!(summary().snoozed_until.is_some());
        h.send(json!({ "type": "settle_thread", "threadId": thread_id, "settled": true }));
        let settled = summary();
        assert_eq!(settled.pinned_at, None);
        assert_eq!(settled.snoozed_until, None);
        assert!(settled.settled_at.is_some());
        assert_eq!(settled.unsettled_at, None);

        // A snooze time in the past is ignored.
        h.send(json!({ "type": "snooze_thread", "threadId": thread_id, "until": "2020-01-01T00:00:00.000Z" }));
        assert_eq!(summary().snoozed_until, None);

        h.send(json!({ "type": "mark_read", "threadId": thread_id, "read": true }));
        let read = summary();
        assert!(ms(&read.last_read_at) >= ms(&read.turn_ended_at));
        h.send(json!({ "type": "mark_read", "threadId": thread_id, "read": false }));
        let unread = summary();
        assert!(ms(&unread.last_read_at) < ms(&unread.turn_ended_at));

        let saved = saved_threads(&store, "workspace").remove(0);
        assert_eq!(saved.sidebar.settled_at, unread.settled_at);
        assert_eq!(saved.sidebar.last_read_at, unread.last_read_at);
        assert_eq!(saved.sidebar.pinned_at, None);
        let again = setup().store(store.clone()).start();
        let restarted = again.state().meta.projects[0]
            .threads
            .iter()
            .find(|thread| thread.id == thread_id)
            .cloned()
            .unwrap()
            .sidebar;
        assert_eq!(restarted.settled_at, unread.settled_at);
        assert_eq!(restarted.last_read_at, unread.last_read_at);
    }

    #[tokio::test]
    async fn brings_a_settled_thread_back_when_the_user_writes_in_it() {
        let h = setup().scripts([quiet(), quiet()]).start();
        h.send(json!({ "type": "send", "text": "first" }));
        h.turn_done().await;
        let thread_id = h.state().meta.thread_id.unwrap();
        h.send(json!({ "type": "settle_thread", "threadId": thread_id, "settled": true }));
        h.send(json!({ "type": "send", "text": "one more thing" }));
        h.turn_done().await;
        let sidebar = &h.state().meta.projects[0].threads[0].sidebar;
        assert_eq!(sidebar.settled_at, None);
        assert!(sidebar.unsettled_at.is_some());
    }

    #[tokio::test]
    async fn saves_a_projects_icon_in_its_project_file_and_its_pin_and_archive_on_this_machine() {
        let store = store_of(vec![workspace_project()]);
        let h = setup().store(store.clone()).start();
        h.send(json!({
            "type": "update_project",
            "projectId": "workspace",
            "icon": { "kind": "lucide", "name": "rocket", "color": "violet" },
            "pinned": true,
        }));
        let sidebar = h.state().meta.projects[0].sidebar.clone();
        assert_eq!(
            sidebar.icon,
            Some(linker_edit_protocol::ProjectIcon::Lucide {
                name: "rocket".into(),
                color: linker_edit_protocol::ProjectIconColor::Violet
            })
        );
        assert!(sidebar.pinned && !sidebar.archived);
        // A malformed icon falls back to the monogram.
        h.send(json!({
            "type": "update_project",
            "projectId": "workspace",
            "icon": { "kind": "lucide", "name": "Not An Icon", "color": "violet" },
            "archived": true,
        }));
        let sidebar = h.state().meta.projects[0].sidebar.clone();
        assert_eq!(
            (sidebar.icon, sidebar.pinned, sidebar.archived),
            (None, true, true)
        );
        let saved = store.project("workspace").unwrap();
        assert_eq!(
            (saved.icon, saved.pinned, saved.archived),
            (None, true, true)
        );
    }
}

mod app_updates {
    use super::*;
    use crate::app_update::{AppUpdates, DesktopUpdater, OnUpdateState};
    use linker_edit_protocol::{DesktopUpdateState, DesktopUpdateStatus};
    use std::sync::OnceLock;
    use std::time::Duration;

    /// The desktop app's updater with a release downloaded; it says when it's asked to restart into it.
    struct Downloaded(Notify);

    #[async_trait::async_trait]
    impl DesktopUpdater for Downloaded {
        fn status(&self) -> DesktopUpdateState {
            DesktopUpdateState {
                status: DesktopUpdateStatus::Downloaded,
                current_version: "2026.1111.2".into(),
                available_version: Some("2026.1112.0".into()),
                downloaded_version: Some("2026.1112.0".into()),
                percent: Some(100.0),
                notes: vec![],
                checked_at: None,
                error: None,
                download_url: None,
                not_applied: None,
            }
        }
        fn on_status(&self, _listener: OnUpdateState) {}
        async fn check(&self) {}
        async fn download(&self) {}
        async fn install(&self) -> bool {
            self.0.notify_one();
            true
        }
        fn cancel(&self) {}
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn restarts_to_update_when_a_page_asks_and_resolve_is_free() {
        let updater = Arc::new(Downloaded(Notify::new()));
        let made: Arc<OnceLock<App>> = Arc::default();
        let app = made.clone();
        let updates = AppUpdates::with_updater(
            updater.clone(),
            Arc::new(move || app.get().is_some_and(App::busy)),
        );
        let h = setup()
            .with(move |options| options.app_updates = Some(updates))
            .start();
        let _ = made.set(h.app.clone());
        // A message is handled with the app locked; asking whether Resolve is busy used to lock it again, and hang.
        let (app, id, runtime) = (h.app.clone(), h.id, tokio::runtime::Handle::current());
        let (done, handled) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _runtime = runtime.enter();
            app.handle_message(
                id,
                message(json!({ "type": "install_app_update" })).unwrap(),
            );
            let _ = done.send(());
        });
        handled
            .recv_timeout(Duration::from_secs(5))
            .expect("install_app_update was handled");
        tokio::time::timeout(Duration::from_secs(5), updater.0.notified())
            .await
            .expect("the updater was asked to restart into the update");
    }
}
