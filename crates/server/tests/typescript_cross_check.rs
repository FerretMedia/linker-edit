// The Rust stores against the TypeScript ones on the same files: the project store, the collections and the edit
// history. typescript/stores.ts writes fixture.json's data with the TypeScript stores and Rust reads it; then Rust
// writes the same data and the TypeScript reads it, and the two sets of files are compared byte for byte.
// Skipped without node or the TypeScript server; remove with the TypeScript server.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use linker_edit_protocol::{CollectionIcon, CollectionKind, Lesson};
use linker_edit_server::collections::store::{
    CollectionStore, FileCollectionStore, NewCollection, SavedCollection,
};
use linker_edit_server::history::history::{
    AgentTurn, GitHistory, History, HistoryOptions, SnapshotReader, SnapshotScope, create_history,
};
use linker_edit_server::history::model::RawSnapshot;
use linker_edit_server::js;
use linker_edit_server::projects::store::{
    FileProjectStore, LoadedProject, NewProject, ProjectStore, SavedProject, SavedThread,
};
use serde_json::{Value, json};

fn script_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("typescript")
}

fn fixture() -> Value {
    serde_json::from_str(&std::fs::read_to_string(script_dir().join("fixture.json")).unwrap())
        .unwrap()
}

fn node_available() -> bool {
    let server =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/server/projects/store.ts");
    server.exists() && Command::new("node").arg("--version").output().is_ok()
}

/// What the TypeScript stores load from dir, after writing the fixture there when mode is "write".
fn typescript(mode: &str, dir: &Path) -> Value {
    let output = Command::new("node")
        .arg(script_dir().join("stores.ts"))
        .arg(mode)
        .arg(dir)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

/// JSON as JSON.stringify writes it, so 2.0 and 2 are one number.
fn normal(value: &Value) -> Value {
    serde_json::from_str(&js::stringify(value)).unwrap()
}

struct FakeResolve(Mutex<RawSnapshot>);

#[async_trait]
impl SnapshotReader for FakeResolve {
    async fn read_snapshot(&self, _scope: Option<SnapshotScope>) -> anyhow::Result<RawSnapshot> {
        Ok(self.0.lock().unwrap().clone())
    }
}

fn snapshot(index: usize) -> RawSnapshot {
    serde_json::from_value(fixture()["snapshots"][index].clone()).unwrap()
}

fn history(dir: &Path, snapshot: RawSnapshot) -> (GitHistory, Arc<FakeResolve>) {
    let resolve = Arc::new(FakeResolve(Mutex::new(snapshot)));
    let history = create_history(HistoryOptions {
        dir: dir.to_path_buf(),
        read_snapshot: resolve.clone(),
        check_project: None,
    });
    (history, resolve)
}

/// A SavedProject as the TypeScript store has it.
fn project_json(project: &SavedProject) -> Value {
    let mut value = json!({
        "id": project.id,
        "dir": project.dir.to_string_lossy(),
        "name": project.name,
        "link": project.link,
        "local": project.local,
        "icon": project.icon,
        "pinned": project.pinned,
        "archived": project.archived,
    });
    if let Some(collection) = &project.collection {
        value["collection"] = json!(collection);
    }
    if project.setup {
        value["setup"] = json!(true);
    }
    value
}

fn collection_json(collection: &SavedCollection) -> Value {
    json!({
        "id": collection.id,
        "kind": collection.kind,
        "name": collection.name,
        "parentId": collection.parent_id,
        "icon": collection.icon,
        "rules": collection.rules,
        "library": collection.library,
        "musicOnlyFromHere": collection.music_only_from_here,
        "autonomous": collection.autonomous,
        "lessons": collection.lessons,
        "edits": collection.edits,
    })
}

struct Rust {
    projects: Vec<LoadedProject>,
    collections: Vec<SavedCollection>,
    examples: Vec<Value>,
    errors: Arc<Mutex<Vec<String>>>,
}

fn open(
    dir: &Path,
) -> (
    FileProjectStore,
    FileCollectionStore,
    Arc<Mutex<Vec<String>>>,
) {
    let errors: Arc<Mutex<Vec<String>>> = Arc::default();
    let (a, b) = (errors.clone(), errors.clone());
    let projects = FileProjectStore::new(
        &dir.join("projects"),
        &dir.join("workspace"),
        move |error| a.lock().unwrap().push(format!("{error:#}")),
    )
    .unwrap();
    let collections = FileCollectionStore::new(dir, move |error| {
        b.lock().unwrap().push(format!("{error:#}"))
    });
    (projects, collections, errors)
}

fn load(dir: &Path) -> Rust {
    let (projects, collections, errors) = open(dir);
    let loaded = collections.load();
    Rust {
        projects: projects.load(),
        examples: loaded
            .iter()
            .map(|collection| json!(collections.examples(&collection.id)))
            .collect(),
        collections: loaded,
        errors,
    }
}

fn promo(rust: &Rust) -> &LoadedProject {
    rust.projects
        .iter()
        .find(|loaded| loaded.project.name.as_deref() == Some("Promo"))
        .unwrap()
}

/// Rust's loads and history of dir against what the TypeScript printed for it.
async fn same_as_typescript(dir: &Path, ts: &Value) {
    // The fixture's two saves, and both timelines found: nothing below compares empty against empty.
    assert_eq!(ts["log"].as_array().map(Vec::len), Some(2));
    assert!(ts["saved"].is_object() && ts["live"].is_object());
    let rust = load(dir);
    let projects: Vec<Value> = rust
        .projects
        .iter()
        .map(
            |loaded| json!({ "project": project_json(&loaded.project), "threads": loaded.threads }),
        )
        .collect();
    assert_eq!(
        normal(&json!(projects)),
        normal(&ts["projects"]),
        "projects"
    );
    let collections: Vec<Value> = rust.collections.iter().map(collection_json).collect();
    assert_eq!(
        normal(&json!(collections)),
        normal(&ts["collections"]),
        "collections"
    );
    assert_eq!(json!(rust.examples), ts["examples"], "examples");

    let (history, _) = history(&promo(&rust).project.dir, snapshot(1));
    let log = history.log(None).await.unwrap();
    assert_eq!(normal(&json!(log)), normal(&ts["log"]), "log");
    let first = log.last().unwrap().commit.clone();
    let saved = history
        .timeline(Some("t2".into()), Some(first))
        .await
        .unwrap();
    assert_eq!(
        normal(&json!(saved)),
        normal(&ts["saved"]),
        "saved timeline"
    );
    let live = history.timeline(None, None).await.unwrap();
    assert_eq!(normal(&json!(live)), normal(&ts["live"]), "live timeline");
    // A save of the same Resolve project finds nothing to save: the files Rust writes are the same bytes.
    assert_eq!(history.record(None).await.unwrap(), None);
    assert_eq!(ts["again"], Value::Null);
    assert_eq!(ts["errors"], json!([]));
    assert!(rust.errors.lock().unwrap().is_empty(), "{:?}", rust.errors);
}

/// The fixture written with the Rust stores, as stores.ts writes it with the TypeScript ones.
async fn write_with_rust(dir: &Path) {
    let fixture = fixture();
    let (projects, collections, errors) = open(dir);
    let fields = &fixture["project"];
    let project = projects.create(NewProject {
        name: serde_json::from_value(fields["name"].clone()).unwrap(),
        link: serde_json::from_value(fields["link"].clone()).unwrap(),
        local: serde_json::from_value(fields["local"].clone()).unwrap(),
        collection: serde_json::from_value(fields["collection"].clone()).unwrap(),
        setup: false,
    });
    let thread: SavedThread = serde_json::from_value(fixture["thread"].clone()).unwrap();
    projects.save_thread(&project, &thread);
    projects.save_project(&SavedProject {
        icon: serde_json::from_value(fixture["icon"].clone()).unwrap(),
        pinned: true,
        ..project.clone()
    });
    let new = |value: &Value, parent_id: Option<String>| NewCollection {
        kind: serde_json::from_value::<CollectionKind>(value["kind"].clone()).unwrap(),
        name: value["name"].as_str().unwrap().into(),
        parent_id,
        icon: serde_json::from_value::<CollectionIcon>(value["icon"].clone()).unwrap(),
        rules: value["rules"].as_str().unwrap().into(),
    };
    let workspace = collections
        .create(new(&fixture["workspace"], None))
        .unwrap();
    let series = collections
        .create(new(&fixture["series"], Some(workspace.id.clone())))
        .unwrap();
    let lesson: Lesson = serde_json::from_value(fixture["lesson"].clone()).unwrap();
    collections.save(&SavedCollection {
        lessons: vec![lesson],
        ..workspace
    });
    collections
        .save_example(&series.id, "Reel 12 - Launch!.mp4", "# Reel 12\n")
        .unwrap();

    let (history, resolve) = history(&project.dir, snapshot(0));
    history.record(None).await.unwrap();
    std::fs::write(project.dir.join("PROJECT.md"), "# Promo\n").unwrap();
    *resolve.0.lock().unwrap() = snapshot(1);
    let turn = &fixture["turn"];
    history
        .record(Some(AgentTurn {
            provider: serde_json::from_value(turn["provider"].clone()).unwrap(),
            prompt: turn["prompt"].as_str().unwrap().into(),
            actions: serde_json::from_value(turn["actions"].clone()).unwrap(),
            summary: turn["summary"].as_str().unwrap().into(),
            thread: turn["thread"].as_str().map(str::to_string),
        }))
        .await
        .unwrap();
    assert!(errors.lock().unwrap().is_empty(), "{errors:?}");
}

/// Every file under dir, relative, with its bytes.
fn files(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let relative = path
                    .strip_prefix(dir)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                found.push((relative, std::fs::read(&path).unwrap()));
            }
        }
    }
    found.sort();
    found
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    String::from_utf8(output.stdout).unwrap()
}

#[tokio::test]
async fn reads_what_typescript_wrote_and_writes_what_typescript_reads() {
    if !node_available() {
        eprintln!("Skipped: no node or no TypeScript server to check against.");
        return;
    }
    // TypeScript writes, Rust reads.
    let from_ts = tempfile::tempdir().unwrap();
    let ts = typescript("write", from_ts.path());
    same_as_typescript(from_ts.path(), &ts).await;

    // Saving what it loaded leaves the TypeScript's files as they were.
    let ts_promo = from_ts.path().join("projects").join("Promo");
    let shared = |dir: &Path| {
        ["linker-edit.json", ".linker-edit/local.json"]
            .map(|file| std::fs::read(dir.join(file)).unwrap())
    };
    let before = shared(&ts_promo);
    let (projects, _, _) = open(from_ts.path());
    for loaded in projects.load() {
        projects.save_project(&loaded.project);
        for thread in &loaded.threads {
            projects.save_thread(&loaded.project, thread);
        }
    }
    assert_eq!(shared(&ts_promo), before);

    // Rust writes, TypeScript reads.
    let from_rust = tempfile::tempdir().unwrap();
    write_with_rust(from_rust.path()).await;
    let ts = typescript("read", from_rust.path());
    same_as_typescript(from_rust.path(), &ts).await;

    // The same data makes the same files: the history and the project's committed files byte for byte.
    let rust_promo = from_rust.path().join("projects").join("Promo");
    let history_files = |dir: &Path| files(&dir.join("resolve"));
    assert_eq!(history_files(&rust_promo), history_files(&ts_promo));
    assert_eq!(shared(&rust_promo), shared(&ts_promo));
    assert_eq!(
        std::fs::read(rust_promo.join("PROJECT.md")).unwrap(),
        std::fs::read(ts_promo.join("PROJECT.md")).unwrap()
    );
    let messages = |dir: &Path| git(dir, &["log", "--format=%an <%ae>%n%B", "--name-status"]);
    assert_eq!(messages(&rust_promo), messages(&ts_promo));
    let rules = |dir: &Path| {
        files(&dir.join("collections"))
            .into_iter()
            .filter(|(file, _)| !file.ends_with("collection.json"))
            .collect::<Vec<_>>()
    };
    assert_eq!(rules(from_rust.path()), rules(from_ts.path()));
}
