// Every message on /ws, decoded from the JSON the TypeScript server and UI send and written back the same.
//
// fixtures/ipad was recorded from the TypeScript server over /ws (apps/desktop/scripts/ipad-fixtures.test.ts on the
// iPad branch); the iPad app decodes the same files. The other fixtures are written from the TypeScript tests.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use linker_edit_protocol::*;
use regex::Regex;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn read(path: &Path) -> Value {
    serde_json::from_str(&fs::read_to_string(path).unwrap())
        .unwrap_or_else(|error| panic!("{}: {error}", path.display()))
}

/// Decodes the JSON as T and checks that encoding it again gives the same JSON, numbers included.
fn round_trip<T: Serialize + DeserializeOwned>(json: &Value) -> T {
    let decoded: T =
        serde_json::from_value(json.clone()).unwrap_or_else(|error| panic!("{error}: {json}"));
    let encoded = serde_json::to_value(&decoded).unwrap();
    assert_eq!(&encoded, json, "encodes differently");
    decoded
}

fn tag(message: &Value) -> String {
    message["type"].as_str().unwrap().to_string()
}

#[test]
fn reads_and_writes_what_the_typescript_server_sent_the_ipad() {
    let mut count = 0;
    for side in ["client", "server"] {
        for entry in fs::read_dir(fixtures().join("ipad").join(side)).unwrap() {
            let json = read(&entry.unwrap().path());
            match side {
                "client" => drop(round_trip::<ClientMessage>(&json)),
                _ => drop(round_trip::<ServerMessage>(&json)),
            }
            count += 1;
        }
    }
    assert_eq!(count, 47);
}

static TYPES: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"\btype: "([a-z_]+)""#).unwrap());

/// The message types protocol.ts declares between `export type <name> =` and the next declaration.
fn declared_types(names: &[&str]) -> BTreeSet<String> {
    let ts = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../apps/desktop/shared/protocol.ts"),
    )
    .unwrap();
    let mut types = BTreeSet::new();
    for name in names {
        let start = ts.find(&format!("export type {name} =")).unwrap();
        let end = ts[start + 1..]
            .find("\nexport ")
            .map_or(ts.len(), |end| start + 1 + end);
        types.extend(
            TYPES
                .captures_iter(&ts[start..end])
                .map(|found| found[1].to_string()),
        );
    }
    types
}

#[test]
fn reads_and_writes_every_client_message() {
    let messages = read(&fixtures().join("client.json"));
    let mut seen = BTreeSet::new();
    for message in messages.as_array().unwrap() {
        let decoded: ClientMessage = round_trip(message);
        // The nested unions are told apart by their type too.
        let expected_group = match tag(message).as_str() {
            t if t.starts_with("library_") => "library",
            "create_collection" | "update_collection" | "remove_lesson" | "remove_example"
            | "setup_collection" | "delete_collection" | "move_project" => "collection",
            "pin_thread" | "settle_thread" | "snooze_thread" | "mark_read" => "sidebar",
            _ => "app",
        };
        let group = match decoded {
            ClientMessage::Library(_) => "library",
            ClientMessage::Collection(_) => "collection",
            ClientMessage::Sidebar(_) => "sidebar",
            _ => "app",
        };
        assert_eq!(group, expected_group, "{message}");
        seen.insert(tag(message));
    }
    let declared = declared_types(&[
        "ClientMessage",
        "LibraryClientMessage",
        "CollectionMessage",
        "SidebarClientMessage",
    ]);
    assert_eq!(seen, declared, "every client message has a fixture");
}

#[test]
fn reads_and_writes_every_server_message() {
    let mut seen = BTreeSet::new();
    let ipad = fs::read_dir(fixtures().join("ipad/server"))
        .unwrap()
        .map(|entry| read(&entry.unwrap().path()));
    let messages = read(&fixtures().join("server.json"));
    for message in messages.as_array().unwrap().iter().cloned().chain(ipad) {
        round_trip::<ServerMessage>(&message);
        seen.insert(tag(&message));
    }
    assert_eq!(
        seen,
        declared_types(&["ServerMessage", "LibraryServerMessage"]),
        "every server message has a fixture"
    );
}

#[test]
fn reads_and_writes_what_the_desktop_app_and_its_server_say() {
    let messages = read(&fixtures().join("desktop.json"));
    let mut seen = BTreeSet::new();
    for message in messages["DesktopToServer"].as_array().unwrap() {
        round_trip::<DesktopToServer>(message);
        seen.insert(tag(message));
    }
    for message in messages["ServerToDesktop"].as_array().unwrap() {
        round_trip::<ServerToDesktop>(message);
        seen.insert(tag(message));
    }
    assert_eq!(
        seen,
        declared_types(&["DesktopToServer", "ServerToDesktop"])
    );
}

#[test]
fn reads_and_writes_every_state_a_value_can_be_in() {
    let values = read(&fixtures().join("values.json"));
    for (name, samples) in values.as_object().unwrap() {
        for sample in samples.as_array().unwrap() {
            match name.as_str() {
                "ChatItem" => drop(round_trip::<ChatItem>(sample)),
                "ResolveStatus" => drop(round_trip::<ResolveStatus>(sample)),
                "PreviewState" => drop(round_trip::<PreviewState>(sample)),
                "TranscriptionStatus" => drop(round_trip::<TranscriptionStatus>(sample)),
                "RemoteAccess" => drop(round_trip::<RemoteAccess>(sample)),
                "LinkerAccount" => drop(round_trip::<LinkerAccount>(sample)),
                "LinkerConnect" => drop(round_trip::<LinkerConnect>(sample)),
                "AppUpdate" => drop(round_trip::<AppUpdate>(sample)),
                "DesktopUpdate" => drop(round_trip::<DesktopUpdate>(sample)),
                "Computers" => drop(round_trip::<Computers>(sample)),
                "PlayableTimeline" => drop(round_trip::<PlayableTimeline>(sample)),
                "ProjectIcon" => drop(round_trip::<ProjectIcon>(sample)),
                "CollectionIcon" => drop(round_trip::<CollectionIcon>(sample)),
                "LibraryKind" => drop(round_trip::<LibraryKind>(sample)),
                "MediaKind" => drop(round_trip::<MediaKind>(sample)),
                "ClipChange" => drop(round_trip::<ClipChange>(sample)),
                other => panic!("no test for {other}"),
            }
        }
    }
}

#[test]
fn leaves_out_optional_fields_but_sends_null_ones() {
    let render = serde_json::to_value(ClientMessage::RenderPreview { timeline_id: None }).unwrap();
    assert_eq!(render, json!({ "type": "render_preview" }));
    let status = serde_json::to_value(ResolveStatus::Connected {
        product: "DaVinci Resolve Studio".into(),
        version: "21.1.2".into(),
        project: None,
        project_id: None,
        database: None,
        timeline: None,
    })
    .unwrap();
    assert_eq!(status["projectId"], Value::Null);
    assert_eq!(status.as_object().unwrap().len(), 7);
}

#[test]
fn tells_null_from_left_out() {
    let parent = |json: Value| match serde_json::from_value(json).unwrap() {
        ClientMessage::Collection(CollectionMessage::UpdateCollection { parent_id, .. }) => {
            parent_id
        }
        other => panic!("{other:?}"),
    };
    assert_eq!(
        parent(json!({ "type": "update_collection", "id": "s" })),
        None
    );
    assert_eq!(
        parent(json!({ "type": "update_collection", "id": "s", "parentId": null })),
        Some(None)
    );
    assert_eq!(
        parent(json!({ "type": "update_collection", "id": "s", "parentId": "w" })),
        Some(Some("w".into()))
    );

    let icon = |json: Value| match serde_json::from_value(json).unwrap() {
        ClientMessage::UpdateProject { changes, .. } => changes.icon,
        other => panic!("{other:?}"),
    };
    assert_eq!(
        icon(json!({ "type": "update_project", "projectId": "p", "pinned": true })),
        None
    );
    assert_eq!(
        icon(json!({ "type": "update_project", "projectId": "p", "icon": null })),
        Some(None)
    );
}

// server/app.test.ts: "saves a project's icon…": a malformed icon falls back to the monogram.
#[test]
fn a_malformed_project_icon_takes_the_icon_away() {
    let message = json!({
        "type": "update_project",
        "projectId": "workspace",
        "icon": { "kind": "lucide", "name": "Not An Icon", "color": "violet" },
        "archived": true,
    });
    let ClientMessage::UpdateProject { changes, .. } = serde_json::from_value(message).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        changes,
        ProjectSidebarChanges {
            icon: Some(None),
            pinned: None,
            archived: Some(true)
        }
    );
    // And it's checked as parseProjectIcon checks it: "ab" becomes "AB".
    let message = json!({ "type": "update_project", "projectId": "p", "icon": { "kind": "monogram", "text": "ab", "color": "red" } });
    let ClientMessage::UpdateProject { changes, .. } = serde_json::from_value(message).unwrap()
    else {
        panic!()
    };
    assert_eq!(
        changes.icon,
        Some(Some(ProjectIcon::Monogram {
            text: "AB".into(),
            color: ProjectIconColor::Red
        }))
    );
}

// server/collections: a new collection with an icon that isn't one gets one from its name instead.
#[test]
fn a_new_collection_with_a_malformed_icon_has_none() {
    let message = json!({ "type": "create_collection", "kind": "workspace", "name": "W", "icon": { "kind": "monogram", "text": "AB", "color": "nope" } });
    let ClientMessage::Collection(CollectionMessage::CreateCollection { icon, .. }) =
        serde_json::from_value(message).unwrap()
    else {
        panic!()
    };
    assert_eq!(icon, None);
}

// The TypeScript server ignored what it didn't know; a message that doesn't decode is ignored the same way.
#[test]
fn refuses_what_isnt_a_message_and_ignores_extra_fields() {
    for json in [
        json!({ "type": "nonsense" }),
        json!({ "type": "set_approval_mode", "mode": "yes" }),
        json!({ "type": "set_default_provider", "provider": "nonsense" }),
        json!({ "type": "update_collection", "id": "s", "icon": { "kind": "monogram", "text": "AB", "color": "nope" } }),
        json!({ "text": "no type" }),
    ] {
        assert!(
            serde_json::from_value::<ClientMessage>(json.clone()).is_err(),
            "{json}"
        );
    }
    let message: ClientMessage =
        serde_json::from_value(json!({ "type": "stop", "extra": 1 })).unwrap();
    assert_eq!(message, ClientMessage::Stop);
}

#[test]
fn keeps_the_constants() {
    assert_eq!(COMPACT_SUGGESTION_MIN_TOKENS, 200_000);
    assert_eq!(
        serde_json::to_value(LIBRARY_KINDS).unwrap(),
        json!(["music", "sfx", "graphics", "luts", "footage", "other"])
    );
    assert_eq!(
        serde_json::to_value(PROJECT_ICON_COLORS)
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        18
    );
}
