// Linker-Edit's Resolve scripts (scripts/*.py), run in this computer's Python against a stand-in Resolve
// (fixtures/DaVinciResolveScript.py), the way Resolve's run_script runs them.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use regex::Regex;
use serde_json::{Value, json};

use super::connection::{ResolveConnection, ResolveError};
use super::run::NODE_TOOLS;
use super::testing::{
    FAKE_RESOLVE_MODULE, PythonConnection, has_python, logged_calls, python_connection,
};
use super::tools::resolve_tools;
use crate::assets::{RESOLVE_SCRIPTS, resolve_script};

fn clip(start: i64, end: i64, id: &str, media: &str) -> Value {
    json!({ "id": id, "name": format!("{media}.mov"), "start": start, "end": end, "source_start": 100, "source_end": 100 + end - start, "media": media })
}

fn state() -> Value {
    json!({
        "project": {
            "name": "Doc",
            "id": "doc-id",
            "current": "Edit",
            "timelines": [
                {
                    "id": "t1",
                    "name": "Edit",
                    "video": [[clip(90000, 90050, "i1", "m1"), clip(90050, 90100, "i2", "m2")]],
                    "audio": [[clip(90000, 90100, "i3", "m1")]]
                },
                {
                    "id": "t2",
                    "name": "AI - Cut",
                    "video": [[clip(90000, 90025, "i4", "m1")]],
                    "audio": [],
                    "markers": [{ "frame": 10, "color": "Blue", "name": "", "note": "", "duration": 1 }]
                }
            ]
        },
        "media": {
            "name": "Master",
            "clips": [{ "id": "m1", "name": "m1.mov", "path": "D:/Footage/m1.mov", "props": { "FPS": 25, "Type": "Video" } }],
            "folders": [{ "name": "Interviews", "clips": [{ "id": "m2", "name": "m2.mov", "path": "D:/Footage/Interviews/m2.mov", "props": { "FPS": 25 } }] }]
        }
    })
}

struct Fake {
    _dir: tempfile::TempDir,
    file: PathBuf,
    connection: Arc<PythonConnection>,
}

impl Fake {
    fn calls(&self) -> Vec<Value> {
        logged_calls(&self.file)
    }

    async fn run(&self, name: &str, input: Value) -> anyhow::Result<Value> {
        self.connection.run_script_value(name, input, 30).await
    }

    async fn tool(&self, name: &str, input: Value) -> anyhow::Result<Value> {
        self.run(&format!("tool_{name}"), input).await
    }
}

fn resolve(state: Value) -> Fake {
    let dir = tempfile::Builder::new()
        .prefix("linker-edit-scripts-")
        .tempdir()
        .unwrap();
    let file = dir.path().join("state.json");
    std::fs::write(&file, state.to_string()).unwrap();
    let connection = python_connection(
        FAKE_RESOLVE_MODULE,
        &[("FAKE_RESOLVE", file.to_str().unwrap())],
    );
    Fake {
        _dir: dir,
        file,
        connection,
    }
}

/// "<kind>: <message>" for a ResolveError, as the TypeScript tests wrote them.
fn refusal(result: anyhow::Result<Value>) -> String {
    let error = result.unwrap_err();
    match error.downcast_ref::<ResolveError>() {
        Some(error) => format!(
            "{}: {}",
            format!("{:?}", error.kind).to_lowercase(),
            error.message
        ),
        None => error.to_string(),
    }
}

fn scripts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src/resolve/scripts")
}

// Blackmagic Cloud's terms (31.5) don't allow scripts to use the service: no script of ours may touch it.
// The agent's run_script is held to the same rule by policy.rs.
#[test]
fn blackmagic_cloud_is_never_used_by_a_resolve_script() {
    let cloud = Regex::new(r"Cloud(?-u:\w)*Project|CLOUD_").unwrap();
    let mut on_disk = Vec::new();
    for entry in std::fs::read_dir(scripts_dir()).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".py") {
            let text = std::fs::read_to_string(entry.path()).unwrap();
            assert!(!cloud.is_match(&text), "{name} uses Blackmagic Cloud");
            on_disk.push(name);
        }
    }
    for (name, text) in RESOLVE_SCRIPTS {
        assert!(!cloud.is_match(text), "{name} uses Blackmagic Cloud");
    }
    // Every script in the folder is in the binary.
    on_disk.sort();
    let embedded: Vec<&str> = RESOLVE_SCRIPTS.iter().map(|(name, _)| *name).collect();
    assert_eq!(on_disk, embedded);
}

#[test]
fn has_a_script_or_a_server_handler_for_every_tool() {
    for tool in resolve_tools() {
        let script = resolve_script(&format!("tool_{}.py", tool.name)).is_some();
        assert!(script || NODE_TOOLS.contains(&tool.name), "{}", tool.name);
    }
}

#[tokio::test]
async fn reads_the_status() {
    if !has_python() {
        return;
    }
    assert_eq!(
        resolve(state()).run("status", json!({})).await.unwrap(),
        json!({
            "state": "connected",
            "product": "DaVinci Resolve Studio",
            "version": "21.1.2",
            "project": "Doc",
            "projectId": "doc-id",
            "database": { "type": "Disk", "name": "Local Database" },
            "timeline": "Edit",
            "page": "edit"
        })
    );
}

#[tokio::test]
async fn lists_timelines_marking_the_current_one_and_the_ai_ones() {
    if !has_python() {
        return;
    }
    let timelines = resolve(state())
        .tool("list_timelines", json!({}))
        .await
        .unwrap();
    let summary: Vec<Value> = timelines
        .as_array()
        .unwrap()
        .iter()
        .map(|t| json!({ "name": t["name"], "current": t["current"], "ai": t["ai"] }))
        .collect();
    assert_eq!(
        summary,
        vec![
            json!({ "name": "Edit", "current": true, "ai": false }),
            json!({ "name": "AI - Cut", "current": false, "ai": true })
        ]
    );
}

#[tokio::test]
async fn reads_a_timelines_clips_with_their_media_narrowed_to_a_track_type_and_frame_range() {
    if !has_python() {
        return;
    }
    let timeline = resolve(state())
        .tool(
            "get_timeline",
            json!({ "track_type": "video", "from_frame": 90060 }),
        )
        .await
        .unwrap();
    assert_eq!(
        timeline["video"][0]["items"],
        json!([{ "id": "i2", "name": "m2.mov", "start": 90050, "end": 90100, "source_start": 100, "source_end": 150, "media": "m2" }])
    );
    assert_eq!(timeline["audio"], json!([]));
    assert_eq!(
        timeline["media"],
        json!({ "m2": { "name": "m2.mov", "path": "D:/Footage/Interviews/m2.mov", "fps": 25 } })
    );
}

#[tokio::test]
async fn lists_media_with_bins_filtered_by_bin() {
    if !has_python() {
        return;
    }
    let media = resolve(state())
        .tool("list_media", json!({ "bin": "Interviews" }))
        .await
        .unwrap();
    assert_eq!(media["bins"], json!(["Master", "Master/Interviews"]));
    let clips = media["clips"].as_array().unwrap();
    assert_eq!(clips.len(), 1);
    assert_eq!(
        [&clips[0]["id"], &clips[0]["bin"], &clips[0]["file_path"]],
        [
            &json!("m2"),
            &json!("Master/Interviews"),
            &json!("D:/Footage/Interviews/m2.mov")
        ]
    );
}

#[tokio::test]
async fn names_new_timelines_ai_unique_and_without_slashes_and_opens_them() {
    if !has_python() {
        return;
    }
    let mut taken = state();
    taken["project"]["timelines"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "t3", "name": "AI - Cut 2" }));
    let fake = resolve(taken);
    let made = fake
        .tool(
            "create_timeline",
            json!({ "name": "AI - Cut", "width": 1080, "height": 1920, "frame_rate": 30 }),
        )
        .await
        .unwrap();
    assert_eq!(made["name"], json!("AI - Cut 3"));
    assert_eq!(made["refused_settings"], json!(["timelineFrameRate"]));
    let calls = fake.calls();
    assert!(calls.contains(&json!(["CreateEmptyTimeline", "AI - Cut 3"])));
    assert_eq!(
        calls.last(),
        Some(&json!(["SetCurrentTimeline", "AI - Cut 3"]))
    );
    assert!(calls.contains(&json!([
        "SetSetting",
        "AI - Cut 3",
        "timelineResolutionWidth",
        "1080"
    ])));
    assert_eq!(
        refusal(
            resolve(state())
                .tool("create_timeline", json!({ "name": " / " }))
                .await
        ),
        "refused: Give the timeline a name."
    );
}

#[tokio::test]
async fn copies_any_timeline_to_an_ai_one() {
    if !has_python() {
        return;
    }
    let fake = resolve(state());
    let copy = fake.tool("duplicate_timeline", json!({})).await.unwrap();
    assert_eq!(copy["name"], json!("AI - Edit copy"));
    assert_eq!(copy["copied_from"], json!({ "id": "t1", "name": "Edit" }));
    assert_eq!(
        fake.calls()[0],
        json!(["DuplicateTimeline", "Edit", "AI - Edit copy"])
    );
}

#[tokio::test]
async fn changes_only_ai_timelines() {
    if !has_python() {
        return;
    }
    let fake = resolve(state());
    for (name, input) in [
        ("add_clips", json!({ "clips": [{ "media": "m1" }] })),
        ("remove_clips", json!({ "item_ids": ["i1"] })),
        (
            "set_clip_properties",
            json!({ "item_ids": ["i1"], "properties": { "ZoomX": 2 } }),
        ),
        ("add_markers", json!({ "markers": [{ "frame": 1 }] })),
    ] {
        let refused = refusal(fake.tool(name, input).await);
        assert!(
            refused.starts_with("refused: \"Edit\" isn't an AI timeline"),
            "{name}: {refused}"
        );
    }
    assert_eq!(fake.calls(), Vec::<Value>::new());
}

#[tokio::test]
async fn puts_history_rows_back_on_a_timeline_as_they_are_adding_missing_tracks() {
    if !has_python() {
        return;
    }
    let fake = resolve(state());
    let result = fake
        .tool(
            "add_clips",
            json!({
                "timeline": "AI - Cut",
                "clips": [
                    { "id": "old", "name": "m1.mov", "media": "m1", "source_start": 10, "source_end": 60, "start": 90100, "end": 90150, "track": 2, "type": "video" },
                    { "media": "m2", "type": "audio" }
                ]
            }),
        )
        .await
        .unwrap();
    let added = result["added"].as_array().unwrap();
    assert_eq!(added.len(), 2);
    let number = |value: &Value| value.as_f64();
    assert_eq!(added[0]["type"], json!("video"));
    assert_eq!(
        [
            number(&added[0]["track"]),
            number(&added[0]["start"]),
            number(&added[0]["end"])
        ],
        [Some(2.0), Some(90100.0), Some(90150.0)]
    );
    assert_eq!(
        [&added[1]["type"], &added[1]["track"]],
        [&json!("audio"), &json!(1)]
    );
    assert_eq!(
        fake.calls(),
        vec![
            json!(["AddTrack", "AI - Cut", "video"]),
            json!(["SetCurrentTimeline", "AI - Cut"]),
            json!([
                "AppendToTimeline",
                "AI - Cut",
                [
                    { "mediaPoolItem": "m1", "startFrame": 10, "endFrame": 60, "recordFrame": 90100, "mediaType": 1, "trackIndex": 2 },
                    { "mediaPoolItem": "m2", "mediaType": 2 }
                ]
            ])
        ]
    );
}

#[tokio::test]
async fn says_which_media_or_clips_dont_exist() {
    if !has_python() {
        return;
    }
    let fake = resolve(state());
    let gone = refusal(
        fake.tool(
            "add_clips",
            json!({ "timeline": "AI - Cut", "clips": [{ "media": "gone" }] }),
        )
        .await,
    );
    assert!(
        gone.starts_with("refused: These media ids aren't in the media pool: gone"),
        "{gone}"
    );
    let off = refusal(
        fake.tool(
            "remove_clips",
            json!({ "timeline": "AI - Cut", "item_ids": ["i1"] }),
        )
        .await,
    );
    assert!(
        off.starts_with("refused: These clips aren't on the timeline: i1"),
        "{off}"
    );
    let nope = refusal(
        fake.tool("get_timeline", json!({ "timeline": "Nope" }))
            .await,
    );
    assert!(
        nope.starts_with("refused: There is no timeline \"Nope\""),
        "{nope}"
    );
}

#[tokio::test]
async fn sets_properties_and_markers_reporting_what_resolve_refused() {
    if !has_python() {
        return;
    }
    let fake = resolve(state());
    let set = fake
        .tool(
            "set_clip_properties",
            json!({ "timeline": "t2", "item_ids": ["i4"], "properties": { "ZoomX": 1.2, "Nonsense": 1 }, "enabled": false }),
        )
        .await
        .unwrap();
    assert_eq!(set["refused"], json!(["i4: Nonsense"]));
    let markers = fake
        .tool(
            "add_markers",
            json!({ "timeline": "AI - Cut", "markers": [{ "frame": 10 }, { "frame": 20, "color": "Red", "name": "Here" }] }),
        )
        .await
        .unwrap();
    assert_eq!(
        [&markers["added"], &markers["refused_frames"]],
        [&json!(1), &json!([10])]
    );
    let calls = fake.calls();
    assert!(calls.contains(&json!(["SetClipEnabled", "i4", false])));
    assert!(calls.contains(&json!(["AddMarker", "AI - Cut", 20, "Red", "Here", "", 1])));
}

#[tokio::test]
async fn imports_into_a_bin_it_makes_then_goes_back_to_the_bin_that_was_open() {
    if !has_python() {
        return;
    }
    let fake = resolve(state());
    assert_eq!(
        fake.tool(
            "import_media",
            json!({ "paths": ["D:/Music/song.wav"], "bin": "AI imports/Music" })
        )
        .await
        .unwrap(),
        json!({ "bin": "Music", "imported": [{ "id": "imported-0", "name": "song.wav", "file_path": "D:/Music/song.wav" }] })
    );
    assert_eq!(
        fake.calls(),
        vec![
            json!(["AddSubFolder", "Master", "AI imports"]),
            json!(["AddSubFolder", "AI imports", "Music"]),
            json!(["SetCurrentFolder", "Music"]),
            json!(["AddItemListToMediaPool", "Music", ["D:/Music/song.wav"]]),
            json!(["SetCurrentFolder", "Master"])
        ]
    );
}

// history::readResolveSnapshot pages through snapshot.py; these check the script's side of it.
#[tokio::test]
async fn saves_the_history_in_pages_when_a_project_takes_long_to_read() {
    if !has_python() {
        return;
    }
    let fake = resolve(state());
    // A budget of 0 reads one timeline per run.
    let first = fake
        .run(
            "snapshot",
            json!({ "ids": null, "current_only": false, "budget": 0 }),
        )
        .await
        .unwrap();
    assert_eq!(
        [
            &first["project"],
            &first["project_id"],
            &first["current_timeline"],
            &first["remaining"]
        ],
        [
            &json!("Doc"),
            &json!("doc-id"),
            &json!("Edit"),
            &json!(["t2"])
        ]
    );
    let names = |page: &Value| {
        page["timelines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(names(&first), [json!("Edit")]);
    let item = &first["timelines"][0]["video"][0]["items"][0];
    assert_eq!(
        [&item["id"], &item["media"]["id"], &item["media"]["path"]],
        [&json!("i1"), &json!("m1"), &json!("D:/Footage/m1.mov")]
    );

    let second = fake
        .run("snapshot", json!({ "ids": ["t2"], "budget": 0 }))
        .await
        .unwrap();
    assert_eq!(names(&second), [json!("AI - Cut")]);
    assert_eq!(second["remaining"], json!([]));
    assert_eq!(
        second["timelines"][0]["markers"],
        json!([{ "frame": 10, "color": "Blue", "name": "", "note": "", "duration": 1 }])
    );
}

#[tokio::test]
async fn reads_only_the_timeline_the_preview_asks_for() {
    if !has_python() {
        return;
    }
    let fake = resolve(state());
    let names = |page: Value| {
        page["timelines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].clone())
            .collect::<Vec<_>>()
    };
    let current = fake
        .run(
            "snapshot",
            json!({ "ids": null, "current_only": true, "budget": 40 }),
        )
        .await
        .unwrap();
    assert_eq!(names(current), [json!("Edit")]);
    let asked = fake
        .run(
            "snapshot",
            json!({ "ids": ["t2"], "current_only": false, "budget": 40 }),
        )
        .await
        .unwrap();
    assert_eq!(names(asked), [json!("AI - Cut")]);
}
