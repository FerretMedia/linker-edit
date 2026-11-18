// Linker-Edit's Resolve tools end to end: the server's side, Resolve's side as real Python against a stand-in
// Resolve (fixtures/DaVinciResolveScript.py), and ffmpeg on generated media. Then restore_timeline over a saved
// timeline, with Resolve's side stood in for: which scripts it runs, with what.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::*;
use crate::resolve::connection::ResolveError;
use crate::resolve::testing::{
    FAKE_RESOLVE_MODULE, PythonConnection, fake_connection, has_python, logged_calls,
    python_connection, test_ffmpeg,
};
use crate::resolve::versions::read_origins;

fn state() -> Value {
    json!({
        "project": {
            "name": "Doc",
            "id": "doc-id",
            "current": "Edit",
            "timelines": [
                { "id": "t1", "name": "Edit", "video": [[{ "id": "i1", "start": 90000, "end": 90050, "media": "m1" }]], "audio": [] },
                {
                    "id": "t2",
                    "name": "AI - Cut",
                    "video": [[{ "id": "i2", "start": 90000, "end": 90025, "media": "m1" }, { "id": "i3", "start": 90025, "end": 90050, "media": "m1" }]],
                    "audio": [[]],
                    "markers": [
                        { "frame": 10, "color": "Blue", "name": "", "note": "", "duration": 1 },
                        { "frame": 20, "color": "Red", "name": "", "note": "", "duration": 1 },
                        { "frame": 30, "color": "Red", "name": "", "note": "", "duration": 1 }
                    ]
                }
            ]
        },
        "media": { "name": "Master", "clips": [{ "id": "m1", "name": "m1.mov", "path": "D:/m1.mov" }], "folders": [{ "name": "Interviews", "clips": [] }] }
    })
}

struct Setup {
    dir: tempfile::TempDir,
    state: PathBuf,
    project: PathBuf,
    connection: Arc<PythonConnection>,
    context: ToolContext,
}

struct Ran {
    json: Value,
    images: Vec<PathBuf>,
}

impl Setup {
    fn calls(&self) -> Vec<Value> {
        logged_calls(&self.state)
    }

    fn dir(&self) -> &Path {
        self.dir.path()
    }

    async fn try_tool(
        &self,
        name: &str,
        input: Value,
        context: &ToolContext,
    ) -> anyhow::Result<Ran> {
        let result = run_resolve_tool(self.connection.clone(), name, input, context).await?;
        Ok(Ran {
            json: serde_json::from_str(&result.text).unwrap(),
            images: result.images,
        })
    }

    async fn tool(&self, name: &str, input: Value) -> Ran {
        self.try_tool(name, input, &self.context).await.unwrap()
    }

    /// The refusal or error: "<kind>: <message>" for Resolve's, the message for others.
    async fn refusal(&self, name: &str, input: Value) -> String {
        self.refusal_in(name, input, &self.context).await
    }

    async fn refusal_in(&self, name: &str, input: Value, context: &ToolContext) -> String {
        let Err(error) = self.try_tool(name, input, context).await else {
            panic!("{name} didn't fail");
        };
        match error.downcast_ref::<ResolveError>() {
            Some(error) => format!(
                "{}: {}",
                format!("{:?}", error.kind).to_lowercase(),
                error.message
            ),
            None => error.to_string(),
        }
    }
}

fn setup() -> Setup {
    let dir = tempfile::Builder::new()
        .prefix("linker-edit-tools-")
        .tempdir()
        .unwrap();
    let state_file = dir.path().join("state.json");
    std::fs::write(&state_file, state().to_string()).unwrap();
    let connection = python_connection(
        FAKE_RESOLVE_MODULE,
        &[("FAKE_RESOLVE", state_file.to_str().unwrap())],
    );
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let mut context = ToolContext::new(&project);
    context.frames_root = Some(dir.path().join("frames"));
    context.poll = Some(Duration::from_millis(1));
    context.ffmpeg = test_ffmpeg();
    Setup {
        dir,
        state: state_file,
        project,
        connection,
        context,
    }
}

#[tokio::test]
async fn exports_a_timeline_into_the_working_folder_named_after_it() {
    if !has_python() {
        return;
    }
    let s = setup();
    let otio = s
        .tool("export_timeline", json!({ "timeline": "AI - Cut" }))
        .await;
    assert_eq!(otio.json["relative"], json!("exports/AI - Cut.otio"));
    assert_eq!(
        std::fs::read_to_string(s.project.join("exports").join("AI - Cut.otio")).unwrap(),
        "exported"
    );
    let edl = s
        .tool(
            "export_timeline",
            json!({ "format": "edl", "name": "cut: list" }),
        )
        .await;
    assert_eq!(edl.json["relative"], json!("exports/cut- list.edl"));
    assert_eq!(
        s.calls(),
        vec![
            json!(["Export", "AI - Cut", "AI - Cut.otio", "otio", "none"]),
            json!(["Export", "Edit", "cut- list.edl", "edl", "none"])
        ]
    );
    let refused = s
        .refusal("export_timeline", json!({ "format": "mp4" }))
        .await;
    assert_eq!(
        refused,
        "Export formats are otio, edl, fcpxml, fcp7xml, aaf, csv, drt."
    );
}

#[tokio::test]
async fn imports_a_timeline_file_as_a_new_ai_timeline_linked_to_the_media_pool() {
    if !has_python() {
        return;
    }
    let s = setup();
    std::fs::write(s.project.join("rebuilt.otio"), "{}").unwrap();
    let result = s
        .tool(
            "import_timeline",
            json!({ "path": "rebuilt.otio", "name": "Rebuilt" }),
        )
        .await;
    assert_eq!(result.json["name"], json!("AI - Rebuilt"));
    assert_eq!(result.json["items"], json!({ "video": 1, "audio": 0 }));
    assert_eq!(
        s.calls(),
        vec![
            json!(["ImportTimelineFromFile", "rebuilt.otio", { "timelineName": "AI - Rebuilt", "importSourceClips": false, "sourceClipsFolders": ["Master", "Interviews"] }]),
            json!(["SetName", "Timeline 1", "AI - Rebuilt"]),
            json!(["SetCurrentTimeline", "AI - Rebuilt"])
        ]
    );
    std::fs::write(s.project.join("clip.mp4"), "").unwrap();
    let wrong = s
        .refusal(
            "import_timeline",
            json!({ "path": "clip.mp4", "name": "X" }),
        )
        .await;
    assert!(wrong.contains("reads .otio"), "{wrong}");
    let missing = s
        .refusal(
            "import_timeline",
            json!({ "path": "missing.otio", "name": "X" }),
        )
        .await;
    assert!(
        missing.contains("There is no file missing.otio"),
        "{missing}"
    );
}

#[tokio::test]
async fn shows_timeline_frames_as_small_jpegs_putting_the_playhead_back() {
    if !has_python() {
        return;
    }
    let s = setup();
    if s.context.ffmpeg.is_none() {
        return;
    }
    let result = s
        .tool(
            "view_frames",
            json!({ "timeline": "AI - Cut", "frames": [90000, 90030] }),
        )
        .await;
    let frames: Vec<Value> = result.json["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| json!({ "frame": frame["frame"], "ok": frame["ok"] }))
        .collect();
    assert_eq!(
        frames,
        vec![
            json!({ "frame": 90000, "ok": true }),
            json!({ "frame": 90030, "ok": true })
        ]
    );
    assert_eq!(result.images.len(), 2);
    for image in &result.images {
        assert_eq!(image.extension().unwrap(), "jpg");
        assert!(image.exists());
        assert!(!image.with_extension("png").exists());
    }
    let playhead: Vec<Value> = s
        .calls()
        .into_iter()
        .filter(|call| call[0] == "SetCurrentTimecode")
        .map(|call| call[2].clone())
        .collect();
    // 25 fps: 90000 is one hour in; the last move puts the playhead back where it was.
    assert_eq!(
        playhead,
        [
            json!("01:00:00:00"),
            json!("01:00:01:05"),
            json!("01:00:00:00")
        ]
    );
}

#[tokio::test]
async fn shows_moments_of_a_media_file_and_finds_its_silences() {
    if !has_python() {
        return;
    }
    let s = setup();
    let Some(ffmpeg) = s.context.ffmpeg.clone() else {
        return;
    };
    let file = s.dir().join("talk.mkv");
    // One second of tone, two of silence, one of tone, with a picture.
    let made = std::process::Command::new(&ffmpeg)
        .args(["-hide_banner", "-loglevel", "error"])
        .args([
            "-f",
            "lavfi",
            "-i",
            "testsrc=size=320x180:rate=25:duration=4",
        ])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono:d=2"])
        .args(["-f", "lavfi", "-i", "sine=frequency=440:duration=1"])
        .args(["-filter_complex", "[1:a][2:a][3:a]concat=n=3:v=0:a=1[a]"])
        .args(["-map", "0:v", "-map", "[a]", "-shortest"])
        .arg(&file)
        .status()
        .unwrap();
    assert!(made.success());
    let frames = s
        .tool("view_frames", json!({ "file": file, "seconds": [0.5, 3] }))
        .await;
    let ok: Vec<&Value> = frames.json["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|frame| &frame["ok"])
        .collect();
    assert_eq!(ok, [&json!(true), &json!(true)]);
    assert_eq!(frames.images.len(), 2);

    let silences = s
        .tool(
            "find_silences",
            json!({ "files": [file, s.dir().join("gone.wav")] }),
        )
        .await;
    let found = &silences.json[0];
    let missing = &silences.json[1];
    assert_eq!(found["silences"].as_array().unwrap().len(), 1);
    let start = found["silences"][0][0].as_f64().unwrap();
    let end = found["silences"][0][1].as_f64().unwrap();
    assert!((start - 1.1).abs() < 0.05, "{start}");
    assert!((end - 2.9).abs() < 0.05, "{end}");
    assert_eq!(found["speech"].as_array().unwrap().len(), 2);
    assert!(
        missing["error"]
            .as_str()
            .unwrap()
            .contains("There is no file"),
        "{missing}"
    );
}

#[tokio::test]
async fn names_switches_and_locks_tracks_of_ai_timelines_only_adding_missing_ones() {
    if !has_python() {
        return;
    }
    let s = setup();
    let refused = s
        .refusal(
            "set_tracks",
            json!({ "tracks": [{ "type": "video", "index": 1, "name": "A" }] }),
        )
        .await;
    assert!(refused.contains("isn't an AI timeline"), "{refused}");
    let result = s
        .tool(
            "set_tracks",
            json!({
                "timeline": "AI - Cut",
                "tracks": [{ "type": "video", "index": 2, "name": "Titles", "locked": true }, { "type": "subtitle", "index": 1, "locked": true }]
            }),
        )
        .await;
    assert_eq!(result.json["refused"], json!(["subtitle 1: locked"]));
    assert_eq!(
        s.calls(),
        vec![
            json!(["AddTrack", "AI - Cut", "video"]),
            json!(["SetTrackName", "video", 2, "Titles"]),
            json!(["SetTrackLock", "video", 2, true]),
            json!(["AddTrack", "AI - Cut", "subtitle"]),
            json!(["SetTrackLock", "subtitle", 1, true])
        ]
    );
}

#[tokio::test]
async fn removes_markers_by_frame_and_by_color() {
    if !has_python() {
        return;
    }
    let s = setup();
    let result = s
        .tool(
            "remove_markers",
            json!({ "timeline": "AI - Cut", "frames": [10, 99], "color": "Red" }),
        )
        .await;
    assert_eq!(
        result.json,
        json!({ "timeline": "AI - Cut", "removed": 3, "no_marker_at": [99] })
    );
}

#[tokio::test]
async fn adds_a_text_title_with_its_text_at_a_frame() {
    if !has_python() {
        return;
    }
    let s = setup();
    let result = s
        .tool(
            "add_title",
            json!({ "timeline": "AI - Cut", "text": "Hello", "frame": 90100, "size": 0.1 }),
        )
        .await;
    for (key, value) in [
        ("id", json!("title-1")),
        ("track", json!(2)),
        ("start", json!(90100)),
        ("end", json!(90225)),
    ] {
        assert_eq!(result.json[key], value, "{key}");
    }
    assert_eq!(
        s.calls(),
        vec![
            json!(["SetCurrentTimeline", "AI - Cut"]),
            json!(["SetCurrentTimecode", "AI - Cut", "01:00:04:00"]),
            json!([
                "InsertFusionTitleIntoTimeline",
                "AI - Cut",
                "Text+",
                "01:00:04:00"
            ]),
            json!(["SetInput", "title-1", "StyledText", "Hello"]),
            json!(["SetInput", "title-1", "Size", 0.1])
        ]
    );
    let refused = s
        .refusal(
            "add_title",
            json!({ "timeline": "AI - Cut", "text": "Hi", "title": "Nope" }),
        )
        .await;
    assert!(
        refused.contains("couldn't insert the title \"Nope\""),
        "{refused}"
    );
}

#[tokio::test]
async fn grades_clips_with_a_lut_a_cdl_or_another_clips_grade() {
    if !has_python() {
        return;
    }
    let s = setup();
    let result = s
        .tool(
            "set_color",
            json!({
                "timeline": "AI - Cut",
                "item_ids": ["i3"],
                "copy_from": "i2",
                "lut": "Film.3dl",
                "cdl": { "slope": 1.1, "offset": [0, 0, 0.01], "saturation": 0.9 }
            }),
        )
        .await;
    assert_eq!(result.json["refused"], json!(["i3: lut"]));
    assert_eq!(
        s.calls(),
        vec![
            json!(["CopyGrades", "i2", ["i3"]]),
            json!(["SetLUT", "i3", 1, "Film.3dl"]),
            json!(["SetCDL", "i3", { "NodeIndex": 1, "Slope": "1.1 1.1 1.1", "Offset": "0.0 0.0 0.01", "Power": "1 1 1", "Saturation": 0.9 }])
        ]
    );
}

#[tokio::test]
async fn files_media_into_a_bin_and_labels_it_without_deleting() {
    if !has_python() {
        return;
    }
    let s = setup();
    let result = s
        .tool(
            "organize_media",
            json!({ "media_ids": ["m1"], "bin": "Selects/Best", "clip_color": "Rainbow", "flag": "Green", "metadata": { "Comments": "Good" } }),
        )
        .await;
    for (key, value) in [
        ("clips", json!(1)),
        ("moved_to", json!("Best")),
        ("refused", json!(["m1: clip_color"])),
    ] {
        assert_eq!(result.json[key], value, "{key}");
    }
    assert_eq!(
        s.calls(),
        vec![
            json!(["AddSubFolder", "Master", "Selects"]),
            json!(["AddSubFolder", "Selects", "Best"]),
            json!(["MoveClips", ["m1"], "Best"]),
            json!(["SetClipColor", "m1", "Rainbow"]),
            json!(["AddFlag", "m1", "Green"]),
            json!(["SetMetadata", "m1", { "Comments": "Good" }])
        ]
    );
}

#[tokio::test]
async fn lists_render_options() {
    if !has_python() {
        return;
    }
    let s = setup();
    let result = s.tool("render_options", json!({ "format": "mp4" })).await;
    assert_eq!(
        result.json["presets"],
        json!(["H.264 Master", "YouTube - 1080p"])
    );
    assert_eq!(result.json["codecs"]["format"], json!("mp4"));
    assert_eq!(result.json["codecs"]["codecs"]["H.264"], json!("H264"));
}

#[tokio::test]
async fn renders_into_the_working_folder_named_after_the_timeline_and_puts_resolve_back() {
    if !has_python() {
        return;
    }
    let s = setup();
    let result = s
        .tool(
            "render_timeline",
            json!({ "timeline": "AI - Cut", "width": 1080, "height": 1920, "from_frame": 90000, "to_frame": 90024 }),
        )
        .await;
    let file = s.project.join("renders").join("AI - Cut.mp4");
    assert_eq!(result.json["file"], json!(file));
    assert_eq!(result.json["timeline"], json!("AI - Cut"));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "rendered");
    let log = s.calls();
    assert!(log.contains(&json!(["SetCurrentRenderFormatAndCodec", "mp4", "H264"])));
    assert!(log.contains(&json!([
        "SetRenderSettings",
        {
            "TargetDir": s.project.join("renders"),
            "CustomName": "AI - Cut",
            "SelectAllFrames": false,
            "MarkIn": 90000,
            "MarkOut": 90024,
            "ExportVideo": true,
            "ExportAudio": true,
            "FormatWidth": 1080,
            "FormatHeight": 1920
        }
    ])));
    // Back to the timeline and render format that were set before.
    assert_eq!(
        log[log.len() - 4..],
        [
            json!(["DeleteRenderJob", "job-1"]),
            json!(["LoadRenderPreset", "Linker-Edit restore"]),
            json!(["SetCurrentRenderFormatAndCodec", "mov", "ProRes422HQ"]),
            json!(["SetCurrentTimeline", "Edit"])
        ]
    );
}

#[tokio::test]
async fn stops_a_render_when_the_user_stops_the_turn_and_still_puts_resolve_back() {
    if !has_python() {
        return;
    }
    let s = setup();
    let mut context = s.context.clone();
    let stopped = CancellationToken::new();
    stopped.cancel();
    context.signal = Some(stopped);
    let refused = s
        .refusal_in(
            "render_timeline",
            json!({ "timeline": "AI - Cut" }),
            &context,
        )
        .await;
    assert_eq!(refused, "Render stopped by the user.");
    let calls = s.calls();
    assert!(calls.iter().any(|call| call[0] == "StopRendering"));
    assert_eq!(calls.last(), Some(&json!(["SetCurrentTimeline", "Edit"])));
    assert!(!s.project.join("renders").join("AI - Cut.mp4").exists());
}

#[tokio::test]
async fn renders_audio_only_and_with_a_preset() {
    if !has_python() {
        return;
    }
    let s = setup();
    let audio = s
        .tool(
            "render_timeline",
            json!({ "timeline": "t2", "audio_only": true, "name": "mix" }),
        )
        .await;
    assert!(audio.json["file"].as_str().unwrap().ends_with("mix.wav"));
    let calls = s.calls();
    assert!(calls.contains(&json!([
        "SetCurrentRenderFormatAndCodec",
        "wav",
        "LinearPCM"
    ])));
    let settings = &calls
        .iter()
        .find(|call| call[0] == "SetRenderSettings")
        .unwrap()[1];
    assert_eq!(
        [&settings["ExportVideo"], &settings["ExportAudio"]],
        [&json!(false), &json!(true)]
    );
    let preset = s
        .refusal("render_timeline", json!({ "preset": "Nope" }))
        .await;
    assert!(preset.contains("no render preset \"Nope\""), "{preset}");
    let codec = s
        .refusal(
            "render_timeline",
            json!({ "format": "mp4", "codec": "ProRes" }),
        )
        .await;
    assert!(codec.contains("no codec \"ProRes\" for mp4"), "{codec}");
}

#[tokio::test]
async fn detects_scene_cuts_on_ai_timelines_only() {
    if !has_python() {
        return;
    }
    let s = setup();
    let refused = s.refusal("detect_scene_cuts", json!({})).await;
    assert!(refused.contains("isn't an AI timeline"), "{refused}");
    let result = s
        .tool("detect_scene_cuts", json!({ "timeline": "AI - Cut" }))
        .await;
    for (key, value) in [
        ("timeline", json!("AI - Cut")),
        ("clips_before", json!(2)),
        ("clips_after", json!(2)),
    ] {
        assert_eq!(result.json[key], value, "{key}");
    }
}

#[test]
fn shortens_long_results_by_capping_their_lists() {
    let long = json!({ "items": (0..100).collect::<Vec<_>>() }).to_string();
    assert_eq!(shorten_text(&long, 20_000), None);
    let shortened = shorten_text(&long, 100).unwrap();
    let first_twenty = (0..20).map(|n| n.to_string()).collect::<Vec<_>>().join(",");
    assert!(
        shortened.starts_with(&format!(
            r#"{{"items":[{first_twenty},"… 80 more items left out"]}}"#
        )),
        "{shortened}"
    );
    assert!(shortened.ends_with(SHORTENED_NOTE));
    let cut_text = shorten_text(&"x".repeat(30), 10).unwrap();
    assert!(
        cut_text.starts_with("xxxxxxxxxx\n… 20 more characters left out."),
        "{cut_text}"
    );
}

#[test]
fn writes_numbers_like_json_stringify() {
    assert_eq!(
        js_numbers(json!({ "a": [1.0, 2.5, 3] })).to_string(),
        r#"{"a":[1,2.5,3]}"#
    );
}

// restore_timeline over a saved timeline, with Resolve's side stood in for.

const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

/// The timeline "Interview" as history::model's storedTimeline saves it: one folder of metadata.json, cuts.json,
/// audio.json and markers.json. The clip "gone" has left the media pool; "Text+" is a title.
fn saved_interview() -> Value {
    let clip = |track: i64, id: &str, name: &str, start: i64, media: Value, extra: Value| {
        let mut clip = json!({
            "track": track, "id": id, "name": name, "start": start, "end": start + 48,
            "source_start": 10, "source_end": 58, "media": media
        });
        if let Value::Object(extra) = extra {
            clip.as_object_mut().unwrap().extend(extra);
        }
        clip
    };
    json!({
        "metadata": {
            "id": "t1", "name": "Interview", "frame_rate": 25, "width": 1080, "height": 1920,
            "start_frame": 90000, "end_frame": 90144, "start_timecode": "01:00:00:00",
            "video_tracks": ["Video 1", "Video 2"], "audio_tracks": ["Audio 1"]
        },
        "cuts": [
            clip(1, "a", "clip a", 90000, json!("m-a"), json!({})),
            clip(1, "b", "clip b", 90048, json!("m-b"), json!({ "properties": { "ZoomX": 1.5 } })),
            clip(1, "gone", "clip gone", 90096, json!("m-gone"), json!({})),
            clip(2, "title", "Text+", 90000, Value::Null, json!({})),
            clip(2, "c", "clip c", 90048, json!("m-c"), json!({ "enabled": false })),
        ],
        "audio": [clip(1, "a", "clip a", 90000, json!("m-a"), json!({}))],
        "markers": [{ "frame": 24, "color": "Red", "name": "Look", "note": "", "duration": 1 }]
    })
}

/// A history with one save, COMMIT, holding the timeline "Interview" (id t1).
struct OneSave;

#[async_trait]
impl SavedTimelines for OneSave {
    async fn read_saved_timeline(
        &self,
        _dir: &Path,
        commit: &str,
        reference: &str,
    ) -> anyhow::Result<Option<SavedTimeline>> {
        if commit != COMMIT || !["t1", "Interview"].contains(&reference) {
            return Ok(None);
        }
        Ok(Some(serde_json::from_value(saved_interview())?))
    }
}

fn restoring_resolve(timelines: Value) -> Arc<crate::resolve::testing::FakeConnection> {
    Arc::new(fake_connection().script(move |name, input| match name {
        "tool_list_timelines" => Ok(timelines.clone()),
        "tool_create_timeline" => {
            let mut created = json!({ "id": "new", "name": format!("AI - {}", input["name"].as_str().unwrap()) });
            if let Some(replaces) = input.get("replaces") {
                created["replaces"] = json!({ "id": replaces, "name": "Interview" });
            }
            Ok(created)
        }
        // Media that left the media pool doesn't go on.
        "tool_add_clips" => {
            let added: Vec<Value> = input["clips"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|clip| clip["media"] != "m-gone")
                .enumerate()
                .map(|(n, clip)| json!({ "id": format!("added-{n}"), "type": clip["type"], "track": clip["track"], "start": clip["start"] }))
                .collect();
            Ok(json!({ "added": added }))
        }
        "tool_set_clip_properties" | "tool_add_markers" => Ok(json!({})),
        other => Err(ResolveError::new(crate::resolve::connection::ResolveErrorKind::Error, format!("No {other} in this test.")).into()),
    }))
}

fn restore_context(dir: &Path) -> ToolContext {
    let mut context = ToolContext::new(dir);
    context.history = Some(Arc::new(OneSave));
    context
}

#[tokio::test]
async fn rebuilds_a_saved_timeline_as_a_new_ai_timeline_settings_clips_on_their_tracks_properties_and_markers()
 {
    let dir = tempfile::tempdir().unwrap();
    let connection = restoring_resolve(json!([{ "id": "t1" }]));
    let result = run_resolve_tool(
        connection.clone(),
        "restore_timeline",
        json!({ "commit": COMMIT, "timeline": "Interview" }),
        &restore_context(dir.path()),
    )
    .await
    .unwrap();
    let result: Value = serde_json::from_str(&result.text).unwrap();
    let calls = connection.scripts();
    assert_eq!(
        connection.script_names(),
        [
            "tool_list_timelines",
            "tool_create_timeline",
            "tool_add_clips",
            "tool_set_clip_properties",
            "tool_set_clip_properties",
            "tool_add_markers"
        ]
    );
    assert_eq!(
        calls[1].args,
        json!({
            "name": "Interview (restored 0123456)",
            "width": 1080,
            "height": 1920,
            "frame_rate": 25,
            "start_timecode": "01:00:00:00",
            "replaces": "t1"
        })
    );
    let clips = calls[2].args["clips"].as_array().unwrap();
    let placed: Vec<String> = clips
        .iter()
        .map(|clip| {
            format!(
                "{} {} {} {}",
                clip["type"].as_str().unwrap(),
                clip["track"],
                clip["start"],
                clip["media"].as_str().unwrap()
            )
        })
        .collect();
    assert_eq!(
        placed,
        [
            "video 1 90000 m-a",
            "video 1 90048 m-b",
            "video 1 90096 m-gone",
            "video 2 90048 m-c",
            "audio 1 90000 m-a"
        ]
    );
    assert_eq!(clips[0]["source_start"], json!(10));
    assert_eq!(
        [&calls[3].args, &calls[4].args],
        [
            &json!({ "timeline": "new", "item_ids": ["added-1"], "properties": { "ZoomX": 1.5 } }),
            &json!({ "timeline": "new", "item_ids": ["added-2"], "properties": {}, "enabled": false })
        ]
    );
    assert_eq!(
        calls[5].args,
        json!({ "timeline": "new", "markers": [{ "frame": 24, "color": "Red", "name": "Look", "note": "", "duration": 1 }] })
    );

    for (key, value) in [
        ("timeline", json!("AI - Interview (restored 0123456)")),
        ("replaces", json!("t1")),
        ("clips", json!({ "saved": 6, "placed": 4 })),
        ("properties_set", json!(2)),
        ("markers", json!(1)),
    ] {
        assert_eq!(result[key], value, "{key}");
    }
    let left_out = result["left_out"].as_array().unwrap();
    assert_eq!(left_out.len(), 2);
    assert!(left_out[0].as_str().unwrap().contains("Text+"));
    assert!(left_out[1].as_str().unwrap().contains("clip gone"));
    // The restored timeline can become the main one once the user approves it.
    assert_eq!(
        serde_json::to_value(read_origins(dir.path())).unwrap(),
        json!({ "new": { "id": "t1", "name": "Interview" } })
    );
}

#[tokio::test]
async fn restores_a_timeline_thats_gone_from_resolve_without_replacing_anything_and_says_when_a_save_has_no_such_timeline()
 {
    let dir = tempfile::tempdir().unwrap();
    let connection = restoring_resolve(json!([]));
    let context = restore_context(dir.path());
    run_resolve_tool(
        connection.clone(),
        "restore_timeline",
        json!({ "commit": COMMIT, "timeline": "t1" }),
        &context,
    )
    .await
    .unwrap();
    assert_eq!(connection.scripts()[1].args.get("replaces"), None);
    assert!(read_origins(dir.path()).is_empty());

    let promo = run_resolve_tool(
        connection.clone(),
        "restore_timeline",
        json!({ "commit": COMMIT, "timeline": "Promo" }),
        &context,
    )
    .await
    .unwrap_err();
    assert!(
        promo
            .to_string()
            .starts_with(&format!("The save {COMMIT} has no timeline \"Promo\"")),
        "{promo}"
    );
    let option = run_resolve_tool(
        connection,
        "restore_timeline",
        json!({ "commit": "--output=x", "timeline": "t1" }),
        &context,
    )
    .await
    .unwrap_err();
    assert!(option.to_string().contains("has no timeline"), "{option}");
}

#[tokio::test]
async fn refuses_a_tool_it_doesnt_have() {
    let error = run_resolve_tool(
        Arc::new(fake_connection()),
        "delete_everything",
        json!({}),
        &ToolContext::new("."),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        "There is no Resolve tool called delete_everything."
    );
}
