#![allow(unused_imports)]
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
#[derive(Clone, Default)]
struct Gate(Arc<Notify>, Arc<AtomicBool>);

impl Gate {
    fn open(&self) {
        self.1.store(true, Ordering::SeqCst);
        self.0.notify_waiters();
    }
    async fn opened(&self) {
        loop {
            let notified = self.0.notified();
            if self.1.load(Ordering::SeqCst) {
                return;
            }
            notified.await;
        }
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
