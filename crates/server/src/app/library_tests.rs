// The port of library/app.test.ts: the library inside the app, the page's messages, and the agents' tools against
// the stand-in Resolve (resolve/fixtures/DaVinciResolveScript.py), running Linker-Edit's real scripts.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use linker_edit_protocol::{LibraryKind, LibraryScope, ResolveDatabase};
use serde_json::{Map, Value, json};
use tokio::sync::oneshot;

use super::testing::*;
use super::*;
use crate::library::folders::{memory_library_store, project_folders};
use crate::library::scan::{Probe, ProbeFile};
use crate::library::testing::temp;
use crate::library::{Library, LibraryOptions};
use crate::projects::store::LoadedProject;
use crate::projects::testing::{memory_project_store, workspace_project};
use crate::resolve::run::{ToolContext, run_resolve_tool};
use crate::resolve::testing::{FAKE_RESOLVE_MODULE, has_python, logged_calls, python_connection};

struct FixedProbe;

#[async_trait]
impl ProbeFile for FixedProbe {
    async fn probe(&self, _: &Path, _: bool) -> anyhow::Result<Probe> {
        Ok(Probe {
            duration: Some(30.0),
            channels: Some(2),
            loudness: Some(-14.0),
        })
    }
}

fn write(file: &Path) {
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(file, "x").unwrap();
}

fn path(file: &Path) -> String {
    file.to_string_lossy().into_owned()
}

struct LibraryHarness {
    h: Harness,
    library: Arc<Library>,
    turn_started: oneshot::Receiver<()>,
    end_turn: oneshot::Sender<()>,
    _cache: tempfile::TempDir,
}

/// Each turn waits until the test lets it end, so the agent's tools can be called while it runs.
fn library_setup(configure: impl FnOnce(&mut AppOptions) + 'static) -> LibraryHarness {
    let cache = temp();
    let library = Arc::new(Library::new(LibraryOptions {
        store: Arc::new(memory_library_store(Vec::new())),
        cache_dir: cache.path().to_path_buf(),
        probe: Arc::new(FixedProbe),
        on_error: None,
    }));
    let (started, turn_started) = oneshot::channel();
    let (end_turn, turn_ends) = oneshot::channel::<()>();
    let in_app = library.clone();
    let h = setup()
        .store(Arc::new(memory_project_store(vec![LoadedProject {
            project: SavedProject {
                name: Some("Doc".into()),
                ..workspace_project()
            },
            threads: Vec::new(),
        }])))
        .scripts([script(move |_| async move {
            let _ = started.send(());
            let _ = turn_ends.await;
            Ok(())
        })])
        .with(move |options| {
            options.check_resolve = Arc::new(|| {
                Box::pin(async {
                    ResolveStatus::Connected {
                        product: "DaVinci Resolve Studio".into(),
                        version: "21".into(),
                        project: Some("Doc".into()),
                        project_id: Some("doc-id".into()),
                        database: Some(ResolveDatabase {
                            r#type: "Disk".into(),
                            name: "Local Database".into(),
                            host: None,
                        }),
                        timeline: Some("Edit".into()),
                    }
                })
            });
            options.library = Some(in_app);
            configure(options);
        })
        .start();
    LibraryHarness {
        h,
        library,
        turn_started,
        end_turn,
        _cache: cache,
    }
}

/// Waits for a library view with this many files.
async fn files_shown(client: &TestClient, count: usize) {
    client
        .until_checked(|_| {
            client
                .views
                .lock()
                .unwrap()
                .last()
                .is_some_and(|view| view.files.len() == count)
        })
        .await;
}

fn input(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap_or_default()
}

#[tokio::test]
async fn adds_folders_to_the_universal_library_and_the_projects_scans_them_and_keeps_whats_noted_on_files()
 {
    let (music, own) = (temp(), temp());
    write(&music.path().join("Upbeat.wav"));
    write(&music.path().join("Calm.wav"));
    write(&own.path().join("Theme.wav"));
    let LibraryHarness { h, library, .. } = library_setup(|_| {});
    h.send(json!({ "type": "library_open", "scope": { "kind": "universal" }, "query": {} }));
    let view = h.client.view();
    assert_eq!(view.scope, LibraryScope::Universal);
    assert!(view.folders.is_empty() && view.files.is_empty());
    assert_eq!(view.total, 0);
    let labels: Vec<String> = view
        .scopes
        .iter()
        .map(|scope| scope.label.clone())
        .collect();
    assert_eq!(labels, ["Universal", "Doc"]);

    h.send(json!({ "type": "library_add_folder", "scope": { "kind": "universal" }, "folder": { "kind": "music", "label": "Songs", "path": path(music.path()), "note": "YouTube only" } }));
    files_shown(&h.client, 2).await;
    let folders = h.client.view().folders;
    assert_eq!(folders.len(), 1);
    assert_eq!(folders[0].folder.label, "Songs");
    assert_eq!(folders[0].folder.note, "YouTube only");
    assert_eq!(folders[0].files, 2);
    assert_eq!(folders[0].scanning, None);
    assert_eq!(folders[0].problem, None);
    let folder_id = folders[0].folder.id.clone();

    h.send(json!({ "type": "library_add_folder", "scope": { "kind": "universal" }, "folder": { "kind": "music", "label": "Again", "path": path(music.path()), "note": "" } }));
    assert!(
        h.client
            .toasts()
            .last()
            .unwrap()
            .contains("already in this library as \"Songs\"")
    );
    h.send(json!({ "type": "library_add_folder", "scope": { "kind": "universal" }, "folder": { "kind": "music", "label": "Gone", "path": path(&music.path().join("nope")), "note": "" } }));
    assert!(
        h.client
            .toasts()
            .last()
            .unwrap()
            .contains("There is no folder")
    );

    h.send(json!({ "type": "library_update_file", "folderId": folder_id, "rel": "Upbeat.wav", "star": true, "tags": ["happy", " Happy ", "intro"], "note": "for the opener" }));
    let first = h.client.view().files[0].clone();
    assert_eq!(first.name, "Upbeat.wav");
    assert!(first.star);
    assert_eq!(first.tags, ["happy", "intro"]);
    assert_eq!(first.note, "for the opener");
    h.send(json!({ "type": "library_open", "scope": { "kind": "universal" }, "query": { "query": "opener" } }));
    let names: Vec<String> = h
        .client
        .view()
        .files
        .iter()
        .map(|file| file.name.clone())
        .collect();
    assert_eq!(names, ["Upbeat.wav"]);
    // Only paths inside the folder.
    h.send(json!({ "type": "library_update_file", "folderId": folder_id, "rel": "../Upbeat.wav", "star": true }));

    h.send(json!({ "type": "library_open", "scope": { "kind": "project", "projectId": "workspace" }, "query": {} }));
    h.send(json!({ "type": "library_add_folder", "scope": { "kind": "project", "projectId": "workspace" }, "folder": { "kind": "music", "label": "Ours", "path": path(own.path()), "note": "" } }));
    files_shown(&h.client, 1).await;
    let view = h.client.view();
    assert_eq!(
        view.scope,
        LibraryScope::Project {
            project_id: "workspace".into()
        }
    );
    let labels: Vec<String> = view
        .folders
        .iter()
        .map(|folder| folder.folder.label.clone())
        .collect();
    assert_eq!(labels, ["Ours"]);
    // Kept in the project's local.json, on this machine only.
    let kept: Vec<String> = project_folders(&h.store.project("workspace").unwrap().local)
        .into_iter()
        .map(|folder| folder.path)
        .collect();
    assert_eq!(kept, [path(&crate::js::resolve(own.path()))]);

    h.send(json!({ "type": "library_remove_folder", "scope": { "kind": "universal" }, "folderId": folder_id }));
    assert!(library.folders().is_empty());
    // Removing a folder forgets it; its files stay.
    assert!(music.path().join("Upbeat.wav").exists());
}

#[tokio::test]
async fn imports_into_library_music_reuses_a_clip_already_in_the_media_pool_and_notes_where_files_were_used()
 {
    if !has_python() {
        return;
    }
    let (music, sfx) = (temp(), temp());
    let song = music.path().join("Song.wav");
    let fresh = music.path().join("Fresh.wav");
    let whoosh = sfx.path().join("Whoosh.wav");
    for file in [&song, &fresh, &whoosh] {
        write(file);
    }
    let outside_dir = temp();
    let outside = outside_dir.path().join("Other.wav");
    write(&outside);

    // Song.wav is in the media pool already, in another bin.
    let resolve_dir = temp();
    let state_file = resolve_dir.path().join("state.json");
    std::fs::write(
        &state_file,
        json!({
            "project": { "name": "Doc", "id": "doc-id", "current": "Edit", "timelines": [{ "id": "t1", "name": "Edit", "video": [[]], "audio": [[]] }] },
            "media": { "name": "Master", "clips": [], "folders": [{ "name": "Music", "clips": [{ "id": "m-song", "name": "Song.wav", "path": path(&song), "props": { "Type": "Audio" } }] }] },
        })
        .to_string(),
    )
    .unwrap();
    let connection =
        python_connection(FAKE_RESOLVE_MODULE, &[("FAKE_RESOLVE", &path(&state_file))]);
    let backups: Arc<Mutex<Vec<PathBuf>>> = Arc::default();
    let backed_up = backups.clone();
    let LibraryHarness {
        h,
        library,
        turn_started,
        end_turn,
        ..
    } = library_setup(move |options| {
        options.resolve_tools = Some(Arc::new(move |tool, input, dir, _| {
            let connection = connection.clone();
            Box::pin(async move {
                let result =
                    run_resolve_tool(connection, &tool, input, &ToolContext::new(dir)).await?;
                Ok(ToolOutput {
                    text: result.text,
                    images: None,
                })
            })
        }));
        options.backup_project = Some(Arc::new(move |dir| {
            backed_up.lock().unwrap().push(dir.clone());
            Box::pin(async move { Ok(path(&dir.join("backups").join("1.drp"))) })
        }));
    });
    library.save_folders(vec![
        LibraryFolder {
            id: "music".into(),
            kind: LibraryKind::Music,
            label: "Songs".into(),
            path: path(music.path()),
            note: "Licensed for YouTube only".into(),
        },
        LibraryFolder {
            id: "sfx".into(),
            kind: LibraryKind::Sfx,
            label: "Whooshes".into(),
            path: path(sfx.path()),
            note: String::new(),
        },
    ]);
    h.app.scan_library().await;

    // Searching only reads, so it works between turns too.
    let search = h
        .app
        .library_tool("library_search", input(json!({ "kind": "music" })))
        .await;
    let found = |file: &Path| json!({ "path": path(file), "kind": "music", "library": "universal", "folder": "Songs", "folder_note": "Licensed for YouTube only", "seconds": 30, "channels": 2, "loudness_lufs": -14 });
    assert_eq!(
        serde_json::from_str::<Value>(&search.text).unwrap(),
        json!({ "results": [found(&fresh), found(&song)], "total": 2 })
    );
    assert_eq!(
        h.app
            .library_tool("library_import", input(json!({ "files": [path(&song)] })))
            .await,
        ToolReply::error("No Linker-Edit turn is running, so Resolve calls are refused.")
    );

    h.send(
        json!({ "type": "send", "text": "put an upbeat song under it and a whoosh on every cut" }),
    );
    turn_started.await.unwrap();
    let result = h
        .app
        .library_tool(
            "library_import",
            input(json!({ "files": [path(&song), path(&fresh), path(&whoosh), path(&outside)] })),
        )
        .await;
    assert!(!result.is_error, "{}", result.text);
    assert_eq!(
        serde_json::from_str::<Value>(&result.text).unwrap(),
        json!({
            "files": [
                { "path": path(&song), "media_id": "m-song", "name": "Song.wav", "bin": "Master/Music", "reused": true },
                { "path": path(&fresh), "media_id": "imported-0", "name": "Fresh.wav", "bin": "Library/Music", "reused": false },
                { "path": path(&whoosh), "media_id": "imported-0", "name": "Whoosh.wav", "bin": "Library/Sound effects", "reused": false },
            ],
            "not_in_library": [path(&outside)],
        })
    );
    let imports: Vec<Value> = logged_calls(&state_file)
        .into_iter()
        .filter(|call| call[0] == "AddItemListToMediaPool" || call[0] == "AddSubFolder")
        .collect();
    // The stand-in starts from its state file on every script, so it makes "Library" again for the second bin.
    assert_eq!(
        imports,
        [
            json!(["AddSubFolder", "Master", "Library"]),
            json!(["AddSubFolder", "Library", "Music"]),
            json!(["AddItemListToMediaPool", "Music", [path(&fresh)]]),
            json!(["AddSubFolder", "Master", "Library"]),
            json!(["AddSubFolder", "Library", "Sound effects"]),
            json!(["AddItemListToMediaPool", "Sound effects", [path(&whoosh)]]),
        ]
    );
    // Changing the project backs it up first, once per turn.
    assert_eq!(*backups.lock().unwrap(), [workspace_project().dir]);

    // A file that's all in the media pool already imports nothing.
    let again = h
        .app
        .library_tool(
            "library_import",
            input(json!({ "files": [path(&song)], "timeline": "AI - Cut" })),
        )
        .await;
    assert_eq!(
        serde_json::from_str::<Value>(&again.text).unwrap(),
        json!({ "files": [{ "path": path(&song), "media_id": "m-song", "name": "Song.wav", "bin": "Master/Music", "reused": true }] })
    );
    let _ = end_turn.send(());

    let used = h
        .app
        .library_tool("library_search", input(json!({ "query": "song" })))
        .await;
    let used: Value = serde_json::from_str(&used.text).unwrap();
    assert_eq!(
        used["results"][0]["used_in"],
        json!(["Doc · Edit", "Doc · AI - Cut"])
    );
}
