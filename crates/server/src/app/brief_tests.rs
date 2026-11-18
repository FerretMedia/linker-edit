// The port of collections/brief.test.ts: setup threads, and the agents' brief tools in the app.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use linker_edit_protocol::{
    ApprovalRequest, ChatItem, CollectionKind, CollectionSummary, LocalLink,
};
use serde_json::{Map, Value, json};

use super::testing::*;
use super::*;
use crate::collections::testing::memory_collection_store;
use crate::projects::store::LoadedProject;
use crate::projects::testing::memory_project_store;
use crate::resolve::versions::{Applied, TimelineRef as VersionRef};
use crate::rules::{AUTONOMOUS_RULES, EDITING_RULES, LEARNING_RULES};

fn project(id: &str) -> SavedProject {
    SavedProject {
        id: id.into(),
        dir: PathBuf::from(format!("/projects/{id}")),
        name: Some(id.into()),
        link: None,
        local: LocalLink::default(),
        icon: None,
        pinned: false,
        archived: false,
        collection: None,
        setup: false,
    }
}

fn input(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

struct Brief {
    h: Harness,
    resolve_calls: Arc<Mutex<Vec<String>>>,
}

/// An app whose agent turns run the scripts in order, each calling the agents' tools as an MCP server would.
fn brief_setup(scripts: Vec<Script>) -> Brief {
    let resolve_calls = Arc::new(Mutex::new(Vec::new()));
    let calls = resolve_calls.clone();
    let h = setup()
        .store(Arc::new(memory_project_store(vec![LoadedProject {
            project: project("ep12"),
            threads: Vec::new(),
        }])))
        .scripts(scripts)
        .with(move |options| {
            options.collections = Some(Arc::new(memory_collection_store(Vec::new())));
            options.resolve_tools = Some(Arc::new(move |tool, _, _, _| {
                calls.lock().unwrap().push(tool);
                Box::pin(async {
                    Ok(ToolOutput {
                        text: "{}".into(),
                        images: None,
                    })
                })
            }));
            options.apply_timeline = Some(Arc::new(|_, timeline_id| {
                Box::pin(async move {
                    Ok(Applied {
                        main: VersionRef {
                            id: timeline_id,
                            name: "Edit".into(),
                        },
                        previous: VersionRef {
                            id: "t1".into(),
                            name: "Edit (before AI)".into(),
                        },
                        was: "AI - Tight".into(),
                    })
                })
            }));
        })
        .start();
    Brief { h, resolve_calls }
}

impl Brief {
    fn by_name(&self, name: &str) -> CollectionSummary {
        self.h
            .state()
            .meta
            .collections
            .into_iter()
            .find(|collection| collection.name == name)
            .unwrap_or_else(|| panic!("no collection {name}"))
    }

    /// Answers the approval that is waiting, and gives back what it showed.
    async fn answer(&self, allow: bool) -> ApprovalRequest {
        self.h
            .client
            .until(|state| !state.meta.approvals.is_empty())
            .await;
        let request = self.h.state().meta.approvals[0].clone();
        self.h
            .send(json!({ "type": "approve", "id": request.id, "allow": allow }));
        request
    }

    /// The workspace "Ferret Media" with the series "Reels", which holds ep12.
    fn in_series(&self) {
        self.h.send(
            json!({ "type": "create_collection", "kind": "workspace", "name": "Ferret Media" }),
        );
        let workspace = self.by_name("Ferret Media").id;
        self.h.send(json!({ "type": "create_collection", "kind": "series", "name": "Reels", "parentId": workspace }));
        let series = self.by_name("Reels").id;
        self.h
            .send(json!({ "type": "move_project", "projectId": "ep12", "collectionId": series }));
    }
}

type Replies = Arc<Mutex<Vec<ToolReply>>>;

mod setup_thread {
    use super::*;

    #[tokio::test]
    async fn opens_once_per_workspace_as_a_project_of_its_own_that_isnt_counted_among_the_workspaces_projects()
     {
        let b = brief_setup(Vec::new());
        let h = &b.h;
        h.send(json!({ "type": "create_collection", "kind": "workspace", "name": "Ferret Media" }));
        h.send(json!({ "type": "setup_collection", "id": b.by_name("Ferret Media").id }));

        let state = h.state();
        let setup_project = state
            .meta
            .projects
            .iter()
            .find(|candidate| candidate.setup)
            .unwrap()
            .clone();
        assert_eq!(setup_project.name, "Ferret Media setup");
        assert_eq!(setup_project.link, None);
        assert_eq!(
            state.meta.project_id.as_deref(),
            Some(setup_project.id.as_str())
        );
        let workspace = b.by_name("Ferret Media");
        assert_eq!(
            workspace.setup_project_id.as_deref(),
            Some(setup_project.id.as_str())
        );
        assert!(workspace.project_ids.is_empty());
        let saved = h.store.project(&setup_project.id).unwrap();
        assert!(saved.setup);
        assert_eq!(saved.collection.unwrap().name, "Ferret Media");

        // Asked again, the same thread opens; a rename shows in its name.
        let thread_id = state.meta.thread_id.clone();
        let other = state
            .meta
            .projects
            .iter()
            .find(|candidate| !candidate.setup)
            .unwrap()
            .threads[0]
            .id
            .clone();
        h.send(json!({ "type": "select_thread", "threadId": other }));
        h.send(json!({ "type": "setup_collection", "id": b.by_name("Ferret Media").id }));
        assert_eq!(h.state().meta.thread_id, thread_id);
        let setups = |h: &Harness| {
            h.state()
                .meta
                .projects
                .into_iter()
                .filter(|candidate| candidate.setup)
                .collect::<Vec<_>>()
        };
        assert_eq!(setups(h).len(), 1);
        h.send(json!({ "type": "update_collection", "id": b.by_name("Ferret Media").id, "name": "Ferret" }));
        assert_eq!(setups(h)[0].name, "Ferret setup");

        // It stays with its workspace, and goes when the workspace is deleted.
        h.send(
            json!({ "type": "move_project", "projectId": setup_project.id, "collectionId": null }),
        );
        assert_eq!(
            h.client.toasts(),
            ["Setup threads stay with what they set up."]
        );
        h.send(json!({ "type": "delete_collection", "id": b.by_name("Ferret").id }));
        assert!(h.state().meta.collections.is_empty());
        let ids: Vec<String> = h
            .state()
            .meta
            .projects
            .into_iter()
            .map(|candidate| candidate.id)
            .collect();
        assert_eq!(ids, ["ep12"]);
    }

    #[tokio::test]
    async fn gets_the_setup_assistants_rules_cant_touch_resolve_and_changes_the_brief_only_when_the_user_accepts()
     {
        let replies: Replies = Arc::default();
        let pushed = replies.clone();
        let b = brief_setup(vec![
            script(move |turn| async move {
                let app = &turn.app;
                let push = |reply| pushed.lock().unwrap().push(reply);
                push(app.resolve_tool("list_media", json!({})).await);
                push(
                    app.resolve_tool("view_frames", json!({ "timeline": "Edit", "frames": [1] }))
                        .await,
                );
                push(
                    app.resolve_tool(
                        "view_frames",
                        json!({ "file": "/videos/reel.mp4", "seconds": [1] }),
                    )
                    .await,
                );
                push(
                    app.library_tool(
                        "library_import",
                        input(json!({ "files": ["/music/a.wav"] })),
                    )
                    .await,
                );
                push(
                    app.brief_tool(
                        "save_example",
                        input(json!({ "name": "Reel 1.mp4", "notes": "34 seconds, 9:16." })),
                    )
                    .await,
                );
                push(
                    app.brief_tool(
                        "save_brief",
                        input(json!({ "rules": "Captions in white.", "summary": "First brief." })),
                    )
                    .await,
                );
                push(
                    app.brief_tool(
                        "save_brief",
                        input(
                            json!({ "rules": "End on the logo.", "summary": "From the examples." }),
                        ),
                    )
                    .await,
                );
                push(
                    app.brief_tool(
                        "save_brief",
                        input(json!({ "for": "series", "series": "Reels", "rules": "30 to 40 seconds, 9:16.", "summary": "The reels format." })),
                    )
                    .await,
                );
                push(
                    app.brief_tool(
                        "save_brief",
                        input(json!({ "rules": "   ", "summary": "Nothing." })),
                    )
                    .await,
                );
                Ok(())
            }),
            quiet(),
        ]);
        let h = &b.h;
        h.send(json!({ "type": "create_collection", "kind": "workspace", "name": "Ferret Media" }));
        h.send(json!({ "type": "setup_collection", "id": b.by_name("Ferret Media").id }));
        h.send(json!({ "type": "send", "text": "My reels are in D:/reels" }));

        // Full access is on, and the brief still waits for the user: also when full access is switched on while it waits.
        assert_eq!(h.state().meta.approval_mode, ApprovalMode::Auto);
        h.client
            .until(|state| !state.meta.approvals.is_empty())
            .await;
        h.send(json!({ "type": "set_approval_mode", "mode": "ask" }));
        h.send(json!({ "type": "set_approval_mode", "mode": "auto" }));
        assert_eq!(h.state().meta.approvals.len(), 1);
        assert_eq!(
            b.answer(false).await.input,
            json!({ "for": "Workspace \"Ferret Media\"", "summary": "First brief.", "was": "", "rules": "Captions in white." })
        );
        assert_eq!(b.by_name("Ferret Media").rules, "");
        assert_eq!(b.answer(true).await.tool, "mcp__linker__save_brief");
        let shown = b.answer(true).await.input;
        assert_eq!(shown["for"], "New series \"Reels\"");
        assert_eq!(shown["was"], "");
        h.turn_done().await;

        let rules = h.rules();
        assert!(
            rules[0].starts_with(
                "You are Linker-Edit's setup assistant. You are learning how Workspace \"Ferret Media\" is edited"
            ),
            "{}",
            rules[0]
        );
        assert!(!rules[0].contains(EDITING_RULES));
        // Only the frames of a file got through to the Resolve tools.
        assert_eq!(*b.resolve_calls.lock().unwrap(), ["view_frames"]);
        let replies = replies.lock().unwrap().clone();
        let errors: Vec<bool> = replies.iter().map(|reply| reply.is_error).collect();
        assert_eq!(
            errors,
            [true, true, false, true, false, true, false, false, true]
        );
        assert!(replies[0].text.contains("setup thread"));
        assert!(replies[5].text.contains("declined"));
        let workspace = b.by_name("Ferret Media");
        assert_eq!(workspace.rules, "End on the logo.\n");
        let examples: Vec<&str> = workspace
            .examples
            .iter()
            .map(|example| example.name.as_str())
            .collect();
        assert_eq!(examples, ["reel-1"]);
        let reels = b.by_name("Reels");
        assert_eq!(reels.kind, CollectionKind::Series);
        assert_eq!(reels.parent_id, Some(workspace.id.clone()));
        assert_eq!(reels.rules, "30 to 40 seconds, 9:16.\n");

        // The next turn's assistant sees the brief as it is now.
        assert!(!h.app.busy());
        h.send(json!({ "type": "send", "text": "Anything else?" }));
        let sessions = h.sessions.clone();
        h.client
            .until(|state| sessions.rules.lock().unwrap().len() == 2 && !state.meta.busy)
            .await;
        let rules = h.rules();
        assert!(rules[1].contains("## Series \"Reels\"\n\n30 to 40 seconds, 9:16."));
        assert!(rules[1].contains("Notes on 1 example video are in"));
    }
}

mod learning {
    use super::*;

    #[tokio::test]
    async fn shows_a_lesson_next_to_the_users_own_words_and_keeps_it_only_when_accepted() {
        let replies: Replies = Arc::default();
        let pushed = replies.clone();
        let b = brief_setup(vec![
            script(move |turn| async move {
                let app = &turn.app;
                let lessons = [
                    json!({ "rule": "Intro at most 3 seconds.", "feedback": "the start drags" }),
                    json!({ "for": "workspace", "rule": "Never use the blue logo.", "feedback": "wrong logo again" }),
                    json!({ "rule": "Shorter.", "feedback": "" }),
                    // A lesson never makes a series; only new rules do.
                    json!({ "for": "series", "series": "Vlog", "rule": "No music.", "feedback": "no music in vlogs" }),
                ];
                for lesson in lessons {
                    let reply = app.brief_tool("add_lesson", input(lesson)).await;
                    pushed.lock().unwrap().push(reply);
                }
                Ok(())
            }),
            quiet(),
        ]);
        b.in_series();
        let h = &b.h;
        h.send(json!({ "type": "send", "text": "the start drags" }));
        assert_eq!(
            b.answer(true).await.input,
            json!({ "for": "Series \"Reels\"", "feedback": "the start drags", "rule": "Intro at most 3 seconds." })
        );
        assert_eq!(
            b.answer(false).await.input["for"],
            "Workspace \"Ferret Media\""
        );
        h.turn_done().await;

        let replies = replies.lock().unwrap().clone();
        let errors: Vec<bool> = replies.iter().map(|reply| reply.is_error).collect();
        assert_eq!(errors, [false, true, true, true]);
        assert!(
            replies[3].text.contains("no series \"Vlog\""),
            "{}",
            replies[3].text
        );
        let lessons = b.by_name("Reels").lessons;
        assert_eq!(lessons.len(), 1);
        assert_eq!(lessons[0].rule, "Intro at most 3 seconds.");
        assert_eq!(lessons[0].feedback, "the start drags");
        assert_eq!(lessons[0].project.as_deref(), Some("ep12"));
        assert!(b.by_name("Ferret Media").lessons.is_empty());

        // The next turn follows it, until the user removes it.
        h.send(json!({ "type": "send", "text": "next" }));
        let sessions = h.sessions.clone();
        h.client
            .until(|state| sessions.rules.lock().unwrap().len() == 2 && !state.meta.busy)
            .await;
        let rules = h.rules();
        assert!(rules[1].contains("- Intro at most 3 seconds."));
        assert!(rules[1].ends_with(LEARNING_RULES));
        let reels = b.by_name("Reels");
        h.send(json!({ "type": "remove_lesson", "id": reels.id, "lessonId": reels.lessons[0].id }));
        assert!(b.by_name("Reels").lessons.is_empty());
    }

    #[tokio::test]
    async fn refuses_the_brief_tools_outside_a_turn_and_for_a_project_with_no_workspace_or_series()
    {
        let replies: Replies = Arc::default();
        let pushed = replies.clone();
        let b = brief_setup(vec![script(move |turn| async move {
            let reply = turn
                .app
                .brief_tool(
                    "add_lesson",
                    input(json!({ "rule": "Intro at most 3 seconds.", "feedback": "the start drags" })),
                )
                .await;
            pushed.lock().unwrap().push(reply);
            Ok(())
        })]);
        let h = &b.h;
        let outside = h
            .app
            .brief_tool("save_brief", input(json!({ "rules": "x", "summary": "y" })))
            .await;
        assert!(outside.is_error);
        assert!(outside.text.contains("No Linker-Edit turn"));
        h.send(json!({ "type": "send", "text": "hi" }));
        h.turn_done().await;
        let replies = replies.lock().unwrap().clone();
        assert!(replies[0].is_error);
        assert!(
            replies[0].text.contains("isn't in a workspace or series"),
            "{}",
            replies[0].text
        );
    }

    #[tokio::test]
    async fn tells_the_agent_to_work_on_its_own_only_where_that_is_switched_on() {
        let b = brief_setup(vec![quiet(), quiet()]);
        b.in_series();
        let h = &b.h;
        let sessions = h.sessions.clone();
        h.send(json!({ "type": "send", "text": "footage is in D:/shoot" }));
        h.client
            .until(|state| sessions.rules.lock().unwrap().len() == 1 && !state.meta.busy)
            .await;
        assert!(!h.rules()[0].contains(AUTONOMOUS_RULES));
        h.send(
            json!({ "type": "update_collection", "id": b.by_name("Reels").id, "autonomous": true }),
        );
        h.send(json!({ "type": "send", "text": "and D:/shoot2" }));
        h.client
            .until(|state| sessions.rules.lock().unwrap().len() == 2 && !state.meta.busy)
            .await;
        assert!(h.rules()[1].contains(AUTONOMOUS_RULES));
    }

    #[tokio::test]
    async fn counts_the_rounds_of_feedback_when_a_timeline_is_approved_and_has_the_agent_learn_from_the_edit()
     {
        let b = brief_setup(vec![quiet(), quiet(), quiet(), quiet()]);
        b.in_series();
        let h = &b.h;
        for text in ["Footage is in D:/shoot", "0:14 is too slow", "Wrong song"] {
            h.send(json!({ "type": "send", "text": text }));
            h.client
                .until(|state| {
                    !state.meta.busy
                        && state
                            .items
                            .iter()
                            .any(|item| matches!(item, ChatItem::User { text: sent, .. } if sent == text))
                })
                .await;
        }
        h.send(json!({ "type": "apply_timeline", "projectId": "ep12", "timelineId": "t2" }));
        let sessions = h.sessions.clone();
        h.client
            .until_checked(|_| sessions.prompts.lock().unwrap().len() == 4)
            .await;
        h.turn_done().await;

        assert!(h.prompts()[3].contains("Learn from this edit:"));
        assert!(h.state().items.iter().any(
            |item| matches!(item, ChatItem::Notice { text, .. } if text.starts_with("Approved:"))
        ));
        // Two messages of feedback after the one that started the edit.
        let edits = b.by_name("Reels").edits;
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0].project, "ep12");
        assert_eq!(edits[0].rounds, 2);
    }
}
