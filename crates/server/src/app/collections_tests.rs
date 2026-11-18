// The app part of collections/collections.test.ts: workspaces and series in the app.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use linker_edit_protocol::{CollectionKind, LocalLink};
use serde_json::json;

use super::testing::*;
use super::*;
use crate::collections::testing::memory_collection_store;
use crate::projects::store::{LoadedProject, ProjectCollection};
use crate::projects::testing::memory_project_store;

fn project(id: &str, dir: PathBuf) -> SavedProject {
    SavedProject {
        id: id.into(),
        dir,
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

#[tokio::test]
async fn sends_collections_to_every_client_and_gives_both_agents_the_projects_rules_at_each_turn() {
    let folder = tempfile::Builder::new()
        .prefix("linker-edit-file-")
        .tempdir()
        .unwrap();
    let dir = folder.path().to_path_buf();
    let prepared: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    let preparing = prepared.clone();
    let h = setup()
        .store(Arc::new(memory_project_store(vec![
            LoadedProject {
                project: project("ep12", dir.clone()),
                threads: Vec::new(),
            },
            LoadedProject {
                project: project("ep11", PathBuf::from("/projects/ep11")),
                threads: Vec::new(),
            },
        ])))
        .with(move |options| {
            options.collections = Some(Arc::new(memory_collection_store(Vec::new())));
            options.prepare_folder = Some(Arc::new(move |dir, text| {
                preparing
                    .lock()
                    .unwrap()
                    .insert(dir.to_string_lossy().into_owned(), text.to_string());
                Ok(())
            }));
        })
        .start();
    let (other, _) = h.connect();
    let prepared_for = |dir: &str| {
        prepared
            .lock()
            .unwrap()
            .get(dir)
            .cloned()
            .unwrap_or_default()
    };
    let ep12_dir = dir.to_string_lossy().into_owned();

    h.send(json!({ "type": "create_collection", "kind": "workspace", "name": "Ferret Media" }));
    let workspace = other.state().meta.collections[0].clone();
    h.send(json!({ "type": "create_collection", "kind": "series", "name": "Podcast", "parentId": workspace.id }));
    let series = other
        .state()
        .meta
        .collections
        .into_iter()
        .find(|collection| collection.kind == CollectionKind::Series)
        .unwrap();
    h.send(json!({ "type": "move_project", "projectId": "ep12", "collectionId": series.id }));
    h.send(json!({ "type": "move_project", "projectId": "ep11", "collectionId": series.id }));
    h.send(json!({ "type": "update_collection", "id": workspace.id, "rules": "Never use the word 'synergy'." }));

    let in_series = other
        .state()
        .meta
        .collections
        .into_iter()
        .find(|collection| collection.id == series.id)
        .unwrap();
    assert_eq!(in_series.project_ids, ["ep12", "ep11"]);
    assert_eq!(
        h.store.project("ep12").unwrap().collection,
        Some(ProjectCollection {
            id: series.id.clone(),
            name: "Podcast".into()
        })
    );
    // Codex's AGENTS.md is rewritten straight away for both projects.
    assert!(prepared_for(&ep12_dir).contains("Never use the word 'synergy'."));
    assert!(prepared_for("/projects/ep11").contains("Never use the word 'synergy'."));

    h.send(json!({ "type": "delete_collection", "id": workspace.id }));
    assert_eq!(
        h.client.toasts(),
        ["Workspace \"Ferret Media\" still has 1 series in it. Move it out first."]
    );
    assert!(other.toasts().is_empty());

    // PROJECT.md saved in the app reaches every client and Codex's AGENTS.md; one that is too long isn't saved.
    h.send(json!({ "type": "save_project_file", "projectId": "ep12", "text": "# Ep 12\n\n## Brief\nThe anniversary episode." }));
    assert_eq!(
        other.files.lock().unwrap().last().cloned(),
        Some((
            "ep12".to_string(),
            dir.join("PROJECT.md").to_string_lossy().into_owned(),
            "# Ep 12\n\n## Brief\nThe anniversary episode.\n".to_string()
        ))
    );
    assert!(
        prepared_for(&ep12_dir)
            .contains("## This project (PROJECT.md in the working folder)\n\n# Ep 12")
    );
    h.send(json!({ "type": "save_project_file", "projectId": "ep12", "text": "x".repeat(20_001) }));
    assert!(
        h.client
            .toasts()
            .last()
            .unwrap()
            .contains("up to 20,000 characters")
    );
    assert!(
        std::fs::read_to_string(dir.join("PROJECT.md"))
            .unwrap()
            .starts_with("# Ep 12")
    );

    // Claude gets them in its system prompt, read when the turn starts.
    h.send(json!({ "type": "send", "text": "Cut the intro" }));
    let sessions = h.sessions.clone();
    h.client
        .until_checked(|_| !sessions.rules.lock().unwrap().is_empty())
        .await;
    let rules = h.rules();
    assert_eq!(rules.len(), 1);
    assert!(rules[0].contains("Never use the word 'synergy'."));
}
