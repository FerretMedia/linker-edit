// Runs Linker-Edit's own Resolve tools (tools.rs) for the agents: most are one script, tool_<name>.py; the
// handlers here do part or all of their work in the server, around Resolve's scripts.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use regex::Regex;
use serde::Deserialize;
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

use super::connection::{ResolveConnection, ResolveErrorKind, error_kind};
use super::media::{
    SilenceOptions, find_ffmpeg, find_silences, frames_dir, js_number_text, media_frame,
    shrink_image,
};
use super::render::{RenderEvent, RenderRequest, render_timeline};
use super::tools::find_resolve_tool;
use super::versions::{TimelineRef, record_origin};

/// A timeline as a save of the edit history holds it: history::model's StoredTimeline, read from its
/// metadata.json, cuts.json, audio.json and markers.json. Only what restore_timeline uses; numbers stay JSON so
/// they go back to Resolve exactly as saved.
#[derive(Debug, Clone, Deserialize)]
pub struct SavedTimeline {
    pub metadata: SavedMetadata,
    #[serde(default)]
    pub cuts: Vec<SavedClip>,
    #[serde(default)]
    pub audio: Vec<SavedClip>,
    #[serde(default)]
    pub markers: Vec<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SavedMetadata {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub width: Option<Value>,
    #[serde(default)]
    pub height: Option<Value>,
    #[serde(default)]
    pub frame_rate: Option<Value>,
    #[serde(default)]
    pub start_timecode: Option<Value>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SavedClip {
    #[serde(default)]
    pub track: Value,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub start: Value,
    #[serde(default)]
    pub source_start: Value,
    #[serde(default)]
    pub source_end: Value,
    /// Media pool item id; None for generators and titles.
    #[serde(default)]
    pub media: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub properties: Option<Map<String, Value>>,
}

/// The edit history, as restore_timeline reads it: history::readSavedTimeline. A trait so the history module
/// (and tests) can supply it.
#[async_trait]
pub trait SavedTimelines: Send + Sync {
    /// The timeline `reference` (its id, or else its exact name) in the save `commit` of the edit history in the
    /// working folder `dir`; None when there is no such save or timeline, including a commit that isn't a hash.
    async fn read_saved_timeline(
        &self,
        dir: &Path,
        commit: &str,
        reference: &str,
    ) -> anyhow::Result<Option<SavedTimeline>>;
}

#[derive(Clone)]
pub struct ToolContext {
    /// The project's working folder, for exports and renders.
    pub dir: PathBuf,
    /// Where frames for view_frames go; temporary. Defaults to linker-edit-frames in the temp folder.
    pub frames_root: Option<PathBuf>,
    /// The ffmpeg to use; media::find_ffmpeg() (next to the app, then PATH) when left out.
    pub ffmpeg: Option<PathBuf>,
    /// How often long jobs are checked on.
    pub poll: Option<Duration>,
    /// Cancelled when the user stops the turn: long jobs such as renders stop with it.
    pub signal: Option<CancellationToken>,
    /// The edit history, for restore_timeline.
    pub history: Option<Arc<dyn SavedTimelines>>,
}

impl ToolContext {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self {
            dir: dir.into(),
            frames_root: None,
            ffmpeg: None,
            poll: None,
            signal: None,
            history: None,
        }
    }

    fn aborted(&self) -> bool {
        self.signal
            .as_ref()
            .is_some_and(CancellationToken::is_cancelled)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolResult {
    pub text: String,
    /// Image files for the agent to see, sent with the text.
    pub images: Vec<PathBuf>,
}

/// What a handler returns: a result, or a result with images for the agent to see.
enum Output {
    Plain(Value),
    WithImages { result: Value, images: Vec<PathBuf> },
}

const MAX_FRAMES: usize = 12;
const EXTENSIONS: [(&str, &str); 7] = [
    ("otio", "otio"),
    ("edl", "edl"),
    ("fcpxml", "fcpxml"),
    ("fcp7xml", "xml"),
    ("aaf", "aaf"),
    ("csv", "csv"),
    ("drt", "drt"),
];
const IMPORTABLE: [&str; 5] = ["otio", "edl", "fcpxml", "xml", "aaf"];

/// Tools whose work starts in the server rather than in a tool_<name>.py script of their own.
pub const NODE_TOOLS: &[&str] = &[
    "duplicate_timeline",
    "create_timeline",
    "restore_timeline",
    "import_media",
    "find_silences",
    "view_frames",
    "export_timeline",
    "import_timeline",
    "detect_scene_cuts",
    "render_timeline",
];

/// A string that isn't blank, as it is.
fn text(value: Option<&Value>) -> Option<&str> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.trim().is_empty())
}

fn num(value: Option<&Value>) -> Option<f64> {
    value
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
}

fn numbers(value: Option<&Value>) -> Vec<f64> {
    match value {
        Some(Value::Array(items)) => items.iter().filter_map(|item| num(Some(item))).collect(),
        _ => Vec::new(),
    }
}

fn strings(value: Option<&Value>) -> Vec<String> {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
        _ => Vec::new(),
    }
}

static UNSAFE_NAME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[\\/:*?"<>|]+"#).unwrap());

fn safe_name(name: &str) -> String {
    let safe = UNSAFE_NAME.replace_all(name, "-").trim().to_string();
    if safe.is_empty() {
        "timeline".to_string()
    } else {
        safe
    }
}

/// String(value), as a template literal writes a JSON value; None is undefined.
fn js_string(value: Option<&Value>) -> String {
    match value {
        None => "undefined".to_string(),
        Some(Value::String(text)) => text.clone(),
        Some(Value::Number(number)) => number
            .as_f64()
            .map_or_else(|| number.to_string(), js_number_text),
        Some(other) => other.to_string(),
    }
}

/// path.resolve(base, file): file against base, with "." and ".." worked out.
fn resolve_path(base: &Path, file: &str) -> PathBuf {
    let mut out = PathBuf::new();
    for part in base.join(file).components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

/// path.isAbsolute: "C:\x", "\x" and "/x" on Windows, "/x" elsewhere.
fn is_absolute(path: &str) -> bool {
    Path::new(path).has_root()
}

fn forward_slashes(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// path.relative for a file inside dir, with forward slashes.
fn relative_slashes(dir: &Path, file: &Path) -> String {
    match file.strip_prefix(dir) {
        Ok(inside) => inside
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/"),
        Err(_) => forward_slashes(file),
    }
}

fn existing_file(file: &str, base: &Path) -> anyhow::Result<PathBuf> {
    let full = resolve_path(base, file);
    if !full.is_file() {
        anyhow::bail!("There is no file {file} on this computer.");
    }
    Ok(full)
}

fn ffmpeg_of(context: &ToolContext) -> anyhow::Result<PathBuf> {
    match &context.ffmpeg {
        Some(ffmpeg) => Ok(ffmpeg.clone()),
        None => find_ffmpeg(),
    }
}

fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value.get(key)
}

fn timeline_ref(value: Option<&Value>) -> Option<TimelineRef> {
    value.and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// Script arguments: `timeline` first when the agent gave one (JSON.stringify leaves undefined out), then the rest.
fn with_timeline(input: &Value, rest: Value) -> Value {
    let mut args = Map::new();
    if let Some(timeline) = input.get("timeline") {
        args.insert("timeline".into(), timeline.clone());
    }
    if let Value::Object(rest) = rest {
        args.extend(rest);
    }
    Value::Object(args)
}

// Both remember what the new AI timeline is a version of, so the user can make it the main one later.
async fn duplicate_timeline(
    connection: &dyn ResolveConnection,
    input: &Value,
    context: &ToolContext,
) -> anyhow::Result<Value> {
    let result = connection
        .run_script_value("tool_duplicate_timeline", input.clone(), 30)
        .await?;
    if let (Some(id), Some(source)) = (
        field(&result, "id").and_then(Value::as_str),
        timeline_ref(field(&result, "copied_from")),
    ) {
        record_origin(&context.dir, id, &source)?;
    }
    Ok(result)
}

async fn create_timeline(
    connection: &dyn ResolveConnection,
    input: Value,
    context: &ToolContext,
) -> anyhow::Result<Value> {
    let result = connection
        .run_script_value("tool_create_timeline", input, 30)
        .await?;
    if let (Some(id), Some(replaces)) = (
        field(&result, "id").and_then(Value::as_str),
        timeline_ref(field(&result, "replaces")),
    ) {
        record_origin(&context.dir, id, &replaces)?;
    }
    Ok(result)
}

async fn restore_timeline(
    connection: &dyn ResolveConnection,
    input: &Value,
    context: &ToolContext,
) -> anyhow::Result<Value> {
    let (Some(commit), Some(reference)) = (text(input.get("commit")), text(input.get("timeline")))
    else {
        anyhow::bail!("Pass the save's commit and the timeline's id or exact name in that save.");
    };
    let Some(history) = &context.history else {
        anyhow::bail!(
            "restore_timeline needs the edit history, which this server didn't pass to the tool."
        );
    };
    let Some(saved) = history
        .read_saved_timeline(&context.dir, commit.trim(), reference)
        .await?
    else {
        anyhow::bail!(
            "The save {commit} has no timeline \"{reference}\". git log -- resolve lists the saves, and git show --stat <commit> their timelines."
        );
    };
    let SavedTimeline {
        metadata,
        cuts,
        audio,
        markers,
    } = saved;
    let short: String = commit.trim().chars().take(7).collect();

    // The saved timeline, if it's still there: the restored one is a new version of it.
    let existing: Vec<Value> = connection
        .run_script("tool_list_timelines", json!({}), 30)
        .await?;
    let original = existing
        .iter()
        .any(|timeline| timeline.get("id").and_then(Value::as_str) == Some(metadata.id.as_str()))
        .then(|| metadata.id.clone());
    let mut settings = Map::new();
    settings.insert(
        "name".into(),
        json!(format!(
            "{} (restored {short})",
            metadata
                .name
                .strip_prefix("AI - ")
                .unwrap_or(&metadata.name)
        )),
    );
    for (key, value) in [
        ("width", &metadata.width),
        ("height", &metadata.height),
        ("frame_rate", &metadata.frame_rate),
        ("start_timecode", &metadata.start_timecode),
    ] {
        if let Some(value) = value {
            settings.insert(key.into(), value.clone());
        }
    }
    if let Some(original) = &original {
        settings.insert("replaces".into(), json!(original));
    }
    let created = create_timeline(connection, Value::Object(settings), context).await?;
    let created_id = created.get("id").cloned().unwrap_or(Value::Null);

    let rows: Vec<(&SavedClip, &str)> = cuts
        .iter()
        .map(|clip| (clip, "video"))
        .chain(audio.iter().map(|clip| (clip, "audio")))
        .collect();
    let has_media = |clip: &SavedClip| clip.media.as_deref().is_some_and(|media| !media.is_empty());
    let placeable: Vec<(&SavedClip, &str)> = rows
        .iter()
        .copied()
        .filter(|(clip, _)| has_media(clip))
        .collect();
    let left_out: Vec<String> = rows
        .iter()
        .filter(|(clip, _)| !has_media(clip))
        .map(|(clip, kind)| {
            format!(
                "{} ({kind} track {}, a title or generator)",
                clip.name,
                js_string(Some(&clip.track))
            )
        })
        .collect();
    let key = |kind: Option<&Value>, track: Option<&Value>, start: Option<&Value>| {
        format!(
            "{}:{}:{}",
            js_string(kind),
            js_string(track),
            js_string(start)
        )
    };
    let mut added: Vec<Value> = Vec::new();
    // A few hundred clips per script, so a long timeline stays inside Resolve's 60 seconds.
    for chunk in placeable.chunks(200) {
        let clips: Vec<Value> = chunk
            .iter()
            .map(|(clip, kind)| {
                json!({
                    "media": clip.media,
                    "source_start": clip.source_start,
                    "source_end": clip.source_end,
                    "start": clip.start,
                    "track": clip.track,
                    "type": kind,
                })
            })
            .collect();
        let result = connection
            .run_script_value(
                "tool_add_clips",
                json!({ "timeline": created_id, "clips": clips }),
                60,
            )
            .await;
        match result {
            Ok(result) => {
                if let Some(Value::Array(items)) = result.get("added") {
                    added.extend(items.iter().cloned());
                }
            }
            Err(error) if error_kind(&error) == Some(ResolveErrorKind::Refused) => {}
            Err(error) => return Err(error),
        }
    }
    let placed: HashMap<String, Value> = added
        .iter()
        .map(|item| {
            let id = item.get("id").cloned().unwrap_or(Value::Null);
            (
                key(item.get("type"), item.get("track"), item.get("start")),
                id,
            )
        })
        .collect();
    let type_value = |kind: &str| Value::String(kind.to_string());
    let missing: Vec<String> = placeable
        .iter()
        .filter(|(clip, kind)| {
            !placed.contains_key(&key(
                Some(&type_value(kind)),
                Some(&clip.track),
                Some(&clip.start),
            ))
        })
        .map(|(clip, kind)| {
            format!(
                "{} ({kind} track {} at frame {}: its media isn't in the media pool any more)",
                clip.name,
                js_string(Some(&clip.track)),
                js_string(Some(&clip.start))
            )
        })
        .collect();

    // Clip properties and disabled clips, one call per distinct set of values.
    let mut groups: Vec<(Map<String, Value>, Vec<Value>)> = Vec::new();
    let mut group_of: HashMap<String, usize> = HashMap::new();
    for (clip, kind) in &placeable {
        let Some(id) = placed.get(&key(
            Some(&type_value(kind)),
            Some(&clip.track),
            Some(&clip.start),
        )) else {
            continue;
        };
        if id.is_null() || (clip.properties.is_none() && clip.enabled != Some(false)) {
            continue;
        }
        let mut change = Map::new();
        change.insert(
            "properties".into(),
            Value::Object(clip.properties.clone().unwrap_or_default()),
        );
        if clip.enabled == Some(false) {
            change.insert("enabled".into(), json!(false));
        }
        let signature = Value::Object(change.clone()).to_string();
        let at = *group_of.entry(signature).or_insert_with(|| {
            groups.push((change, Vec::new()));
            groups.len() - 1
        });
        groups[at].1.push(id.clone());
    }
    let mut refused: Vec<Value> = Vec::new();
    for (change, ids) in &groups {
        let mut args = Map::new();
        args.insert("timeline".into(), created_id.clone());
        args.insert("item_ids".into(), json!(ids));
        args.extend(change.clone());
        let result = connection
            .run_script_value("tool_set_clip_properties", Value::Object(args), 30)
            .await?;
        if let Some(Value::Array(items)) = result.get("refused") {
            refused.extend(items.iter().cloned());
        }
    }
    if !markers.is_empty() {
        connection
            .run_script_value(
                "tool_add_markers",
                json!({ "timeline": created_id, "markers": markers }),
                30,
            )
            .await?;
    }

    let mut out = Map::new();
    out.insert(
        "timeline".into(),
        created.get("name").cloned().unwrap_or(Value::Null),
    );
    out.insert("id".into(), created_id);
    out.insert("replaces".into(), json!(original));
    out.insert(
        "clips".into(),
        json!({ "saved": rows.len(), "placed": added.len() }),
    );
    out.insert(
        "properties_set".into(),
        json!(groups.iter().map(|(_, ids)| ids.len()).sum::<usize>()),
    );
    out.insert("markers".into(), json!(markers.len()));
    if !left_out.is_empty() || !missing.is_empty() {
        out.insert("left_out".into(), json!([left_out, missing].concat()));
    }
    if !refused.is_empty() {
        out.insert("refused_properties".into(), Value::Array(refused));
    }
    out.insert(
        "note".into(),
        json!("Color grades, effects, titles and generators aren't in the edit history, so they aren't restored. Read the timeline back with get_timeline."),
    );
    Ok(Value::Object(out))
}

async fn import_media(connection: &dyn ResolveConnection, input: &Value) -> anyhow::Result<Value> {
    let paths = strings(input.get("paths"));
    if paths.is_empty() {
        anyhow::bail!("Pass the files to import in paths.");
    }
    let bad: Vec<&str> = paths
        .iter()
        .filter(|file| !is_absolute(file) || !Path::new(file).exists())
        .map(String::as_str)
        .collect();
    if !bad.is_empty() {
        anyhow::bail!(
            "These aren't files on this computer: {}. Pass absolute paths.",
            bad.join(", ")
        );
    }
    connection
        .run_script_value("tool_import_media", input.clone(), 60)
        .await
}

async fn find_silences_tool(input: &Value, context: &ToolContext) -> anyhow::Result<Value> {
    let files = strings(input.get("files"));
    if files.is_empty() {
        anyhow::bail!("Pass at least one file path in files.");
    }
    let ffmpeg = ffmpeg_of(context)?;
    let options = SilenceOptions {
        min_silence: num(input.get("min_silence")).unwrap_or(0.5),
        threshold_db: num(input.get("threshold_db")).unwrap_or(-35.0),
        padding: num(input.get("padding")).unwrap_or(0.1),
    };
    let tasks: Vec<_> = files
        .into_iter()
        .map(|file| {
            let ffmpeg = ffmpeg.clone();
            let dir = context.dir.clone();
            tokio::spawn(async move {
                let found = async {
                    let full = existing_file(&file, &dir)?;
                    find_silences(&ffmpeg, &full, options).await
                }
                .await;
                match found {
                    Ok(silences) => serde_json::to_value(silences).unwrap_or(Value::Null),
                    Err(error) => json!({ "file": file, "error": error.to_string() }),
                }
            })
        })
        .collect();
    let mut out = Vec::new();
    for task in tasks {
        out.push(task.await?);
    }
    Ok(Value::Array(out))
}

async fn view_frames(
    connection: &dyn ResolveConnection,
    input: &Value,
    context: &ToolContext,
) -> anyhow::Result<Output> {
    let root = context
        .frames_root
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().join("linker-edit-frames"));
    if let Some(file) = text(input.get("file")) {
        let seconds: Vec<f64> = numbers(input.get("seconds"))
            .into_iter()
            .take(MAX_FRAMES)
            .collect();
        if seconds.is_empty() {
            anyhow::bail!("Pass the moments to see in seconds.");
        }
        let ffmpeg = ffmpeg_of(context)?;
        let source = existing_file(file, &context.dir)?;
        let dir = frames_dir(&root)?;
        let tasks: Vec<_> = seconds
            .into_iter()
            .enumerate()
            .map(|(index, second)| {
                let (ffmpeg, source) = (ffmpeg.clone(), source.clone());
                let out = dir.join(format!("frame-{index}.jpg"));
                tokio::spawn(async move {
                    let ok = media_frame(&ffmpeg, &source, second, &out).await;
                    (second, out, ok)
                })
            })
            .collect();
        let mut frames = Vec::new();
        let mut images = Vec::new();
        for task in tasks {
            let (second, out, ok) = task.await?;
            frames.push(json!({ "seconds": second, "file": out, "ok": ok }));
            if ok {
                images.push(out);
            }
        }
        return Ok(Output::WithImages {
            result: json!({ "file": source, "frames": frames }),
            images,
        });
    }
    let frames: Vec<f64> = numbers(input.get("frames"))
        .into_iter()
        .take(MAX_FRAMES)
        .collect();
    if frames.is_empty() {
        anyhow::bail!("Pass timeline frames, or a file with seconds.");
    }
    let dir = frames_dir(&root)?;
    #[derive(Deserialize)]
    struct Still {
        frame: Value,
        file: String,
        ok: bool,
    }
    #[derive(Deserialize)]
    struct Stills {
        timeline: Value,
        stills: Vec<Still>,
    }
    let args = with_timeline(
        input,
        json!({ "frames": frames, "dir": forward_slashes(&dir) }),
    );
    let stills: Stills = connection.run_script("timeline_stills", args, 60).await?;
    // Without ffmpeg the agent gets Resolve's full-size stills.
    let ffmpeg = ffmpeg_of(context).ok();
    let mut shown = Vec::new();
    let mut images = Vec::new();
    let mut missing = false;
    for still in stills.stills {
        let exported = still.ok && Path::new(&still.file).exists();
        let file = if exported {
            Some(shrink_image(ffmpeg.as_deref(), Path::new(&still.file)).await)
        } else {
            None
        };
        missing |= !exported;
        if let Some(file) = &file {
            images.push(file.clone());
        }
        shown.push(json!({ "frame": still.frame, "ok": exported, "file": file }));
    }
    let mut result = json!({ "timeline": stills.timeline, "frames": shown });
    if missing {
        result["note"] = json!(
            "Resolve exported no still for some frames: outside the timeline, or Resolve is busy."
        );
    }
    Ok(Output::WithImages { result, images })
}

async fn export_timeline(
    connection: &dyn ResolveConnection,
    input: &Value,
    context: &ToolContext,
) -> anyhow::Result<Value> {
    let format = text(input.get("format")).unwrap_or("otio");
    let Some((_, extension)) = EXTENSIONS.iter().find(|(name, _)| *name == format) else {
        let names: Vec<&str> = EXTENSIONS.iter().map(|(name, _)| *name).collect();
        anyhow::bail!("Export formats are {}.", names.join(", "));
    };
    let dir = context.dir.join("exports");
    std::fs::create_dir_all(&dir)?;
    let mut args = with_timeline(
        input,
        json!({ "format": format, "dir": forward_slashes(&dir), "extension": extension }),
    );
    if let Some(name) = text(input.get("name")) {
        args["name"] = json!(safe_name(name));
    }
    let mut result = connection
        .run_script_value("tool_export_timeline", args, 60)
        .await?;
    let path = result
        .get("path")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let file = resolve_path(Path::new(""), path);
    if !file.exists() {
        anyhow::bail!("Resolve said it exported the timeline, but the file isn't there.");
    }
    result["path"] = json!(file);
    result["relative"] = json!(relative_slashes(&context.dir, &file));
    Ok(result)
}

async fn import_timeline(
    connection: &dyn ResolveConnection,
    input: &Value,
    context: &ToolContext,
) -> anyhow::Result<Value> {
    let file = existing_file(text(input.get("path")).unwrap_or(""), &context.dir)?;
    let extension = file
        .extension()
        .map(|extension| extension.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    if !IMPORTABLE.contains(&extension.as_str()) {
        anyhow::bail!("import_timeline reads .otio, .edl, .fcpxml, .xml and .aaf files.");
    }
    let mut args = input.clone();
    if let Value::Object(args) = &mut args {
        args.insert("path".into(), json!(file));
    }
    connection
        .run_script_value("tool_import_timeline", args, 60)
        .await
}

static SCENE_CUTS_RUNNING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new("(?i)^(Initializing|Analyzing|Processing|Running|InProgress|In Progress|Cutting)$")
        .unwrap()
});

async fn detect_scene_cuts(
    connection: &dyn ResolveConnection,
    input: &Value,
    context: &ToolContext,
) -> anyhow::Result<Value> {
    let started = match connection
        .run_script_value("scene_cuts_start", input.clone(), 60)
        .await
    {
        Ok(started) => Some(started),
        // Resolve may still be cutting when the script runs out of time; the status below says.
        Err(error) if error_kind(&error) == Some(ResolveErrorKind::Timeout) => None,
        Err(error) => return Err(error),
    };
    let timeline = started
        .as_ref()
        .and_then(|started| started.get("timeline_id"))
        .filter(|id| !id.is_null())
        .or_else(|| input.get("timeline"))
        .cloned();
    let args = match timeline {
        Some(timeline) => json!({ "timeline": timeline }),
        None => json!({}),
    };
    let deadline = Instant::now() + Duration::from_secs(15 * 60);
    let status = loop {
        let status = connection
            .run_script_value("scene_cuts_status", args.clone(), 10)
            .await?;
        let job = status
            .pointer("/status/JobStatus")
            .and_then(Value::as_str)
            .unwrap_or("");
        if !SCENE_CUTS_RUNNING.is_match(job) || Instant::now() > deadline {
            break status;
        }
        // Resolve finishes the cut by itself; a stopped turn only stops waiting for it.
        if context.aborted() {
            anyhow::bail!("Stopped by the user while Resolve was still detecting scene cuts.");
        }
        tokio::time::sleep(context.poll.unwrap_or(Duration::from_secs(2))).await;
    };
    let before = started
        .as_ref()
        .and_then(|started| started.get("before"))
        .cloned()
        .unwrap_or(Value::Null);
    Ok(json!({
        "timeline": status.get("name"),
        "clips_before": before,
        "clips_after": status.get("clips"),
        "status": status.get("status"),
    }))
}

async fn render_timeline_tool(
    connection: Arc<dyn ResolveConnection>,
    input: &Value,
    context: &ToolContext,
) -> anyhow::Result<Value> {
    let audio_only = input.get("audio_only") == Some(&Value::Bool(true));
    let dir = match text(input.get("dir")) {
        Some(dir) => {
            if !is_absolute(dir) {
                anyhow::bail!("dir must be an absolute folder.");
            }
            PathBuf::from(dir)
        }
        None => context.dir.join("renders"),
    };
    let owned = |value: Option<&str>| value.map(str::to_string);
    let mut request = RenderRequest {
        timeline_id: owned(text(input.get("timeline"))),
        dir,
        name: owned(text(input.get("name"))),
        preset: owned(text(input.get("preset"))),
        format: owned(text(input.get("format"))).or_else(|| audio_only.then(|| "wav".to_string())),
        codec: owned(text(input.get("codec"))),
        width: num(input.get("width")),
        height: num(input.get("height")),
        from_frame: num(input.get("from_frame")),
        to_frame: num(input.get("to_frame")),
        ..Default::default()
    };
    if audio_only {
        request.audio = Some(true);
        request.video = Some(false);
    }
    let job = render_timeline(
        connection,
        request,
        Arc::new(|_| {}),
        context.poll.unwrap_or(Duration::from_secs(1)),
    );
    // An hour at most, so a stuck render doesn't hold Resolve forever; stopping the turn stops it too.
    let stop = job.stop_token();
    if context.aborted() {
        stop.cancel();
    }
    let signal = context.signal.clone().unwrap_or_default();
    let watcher = tokio::spawn(async move {
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(60 * 60)) => stop.cancel(),
            _ = signal.cancelled() => stop.cancel(),
            _ = stop.cancelled() => {}
        }
    });
    let started = Instant::now();
    let result = job.finished().await;
    watcher.abort();
    match result {
        RenderEvent::Done { file, timeline } => Ok(json!({
            "file": file,
            "timeline": timeline.name,
            "seconds": (started.elapsed().as_secs_f64()).round(),
        })),
        RenderEvent::Error {
            stopped: Some(true),
            ..
        } if context.aborted() => anyhow::bail!("Render stopped by the user."),
        RenderEvent::Error { message, .. } => anyhow::bail!(message),
        _ => anyhow::bail!("The render didn't finish."),
    }
}

/// Runs one of Linker-Edit's Resolve tools. Its result is compact JSON, shortened when long.
pub async fn run_resolve_tool(
    connection: Arc<dyn ResolveConnection>,
    name: &str,
    input: Value,
    context: &ToolContext,
) -> anyhow::Result<ToolResult> {
    let Some(tool) = find_resolve_tool(name) else {
        anyhow::bail!("There is no Resolve tool called {name}.");
    };
    let conn = connection.as_ref();
    let output = match name {
        "duplicate_timeline" => Output::Plain(duplicate_timeline(conn, &input, context).await?),
        "create_timeline" => Output::Plain(create_timeline(conn, input, context).await?),
        "restore_timeline" => Output::Plain(restore_timeline(conn, &input, context).await?),
        "import_media" => Output::Plain(import_media(conn, &input).await?),
        "find_silences" => Output::Plain(find_silences_tool(&input, context).await?),
        "view_frames" => view_frames(conn, &input, context).await?,
        "export_timeline" => Output::Plain(export_timeline(conn, &input, context).await?),
        "import_timeline" => Output::Plain(import_timeline(conn, &input, context).await?),
        "detect_scene_cuts" => Output::Plain(detect_scene_cuts(conn, &input, context).await?),
        "render_timeline" => {
            Output::Plain(render_timeline_tool(connection.clone(), &input, context).await?)
        }
        _ => Output::Plain(
            conn.run_script_value(&format!("tool_{name}"), input, tool.timeout.unwrap_or(30))
                .await?,
        ),
    };
    let (result, images) = match output {
        Output::Plain(result) => (result, Vec::new()),
        Output::WithImages { result, images } => (result, images),
    };
    let json = js_numbers(result).to_string();
    Ok(ToolResult {
        text: shorten_text(&json, MAX_TOOL_OUTPUT_CHARS).unwrap_or(json),
        images,
    })
}

/// JSON as JSON.parse and JSON.stringify make it: whole numbers without ".0".
pub(crate) fn js_numbers(value: Value) -> Value {
    match value {
        Value::Number(number) if number.is_f64() => match number.as_f64() {
            Some(float) if float.fract() == 0.0 && float.abs() <= 9_007_199_254_740_991.0 => {
                json!(float as i64)
            }
            _ => Value::Number(number),
        },
        Value::Array(items) => Value::Array(items.into_iter().map(js_numbers).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, js_numbers(value)))
                .collect(),
        ),
        other => other,
    }
}

// The port of providers/shorten.ts's shortenText, which this module needs. Resolve tools can return thousands of
// timeline items or transcript lines in one call. All of it would stay in the agent's context for the rest of
// the chat, so long results are cut first.
// TODO(hub): use providers::shorten::shorten_text once the providers port lands, and drop this copy.

/// About 5,000 tokens.
const MAX_TOOL_OUTPUT_CHARS: usize = 20_000;
const ITEM_LIMITS: [usize; 5] = [50, 20, 10, 5, 2];
const SHORTENED_NOTE: &str = "Linker-Edit shortened this result to save context. Ask again for a narrower part (one track, a range, a filter) to see what was left out.";

/// A JSON result with its long lists capped, or a plain cut when it isn't JSON; None when it fits. Lengths count
/// characters.
fn shorten_text(text: &str, max: usize) -> Option<String> {
    if text.chars().count() <= max {
        return None;
    }
    let Ok(parsed) = serde_json::from_str::<Value>(text) else {
        return Some(cut(text, max));
    };
    for limit in ITEM_LIMITS {
        // Compact JSON: indentation would cost tokens too.
        let shortened = cap_lists(&parsed, limit).to_string();
        if shortened.chars().count() <= max {
            return Some(format!("{shortened}\n{SHORTENED_NOTE}"));
        }
    }
    Some(cut(
        &cap_lists(&parsed, ITEM_LIMITS[ITEM_LIMITS.len() - 1]).to_string(),
        max,
    ))
}

fn cut(text: &str, max: usize) -> String {
    let kept: String = text.chars().take(max).collect();
    let left = text.chars().count() - max;
    format!("{kept}\n… {left} more characters left out. {SHORTENED_NOTE}")
}

fn cap_lists(value: &Value, limit: usize) -> Value {
    match value {
        Value::Array(items) => {
            let mut kept: Vec<Value> = items
                .iter()
                .take(limit)
                .map(|item| cap_lists(item, limit))
                .collect();
            if items.len() > limit {
                kept.push(json!(format!(
                    "… {} more items left out",
                    items.len() - limit
                )));
            }
            Value::Array(kept)
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(key, item)| (key.clone(), cap_lists(item, limit)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests;
