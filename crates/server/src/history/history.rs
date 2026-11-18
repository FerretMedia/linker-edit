// The edit history: Resolve's open project saved into a Git repository in the project's folder, before and after
// each agent turn, with the git CLI as the TypeScript server ran it. The repository, its file layout and its commit
// messages are the same, so the Node and Rust apps keep one history between them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};

use anyhow::{Context, anyhow, bail};
use async_trait::async_trait;
use linker_edit_protocol::{HistoryEntry, MediaKind, PlayableTimeline, Provider};
use regex::Regex;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::OnceCell;

use super::diff::{describe_changes, timeline_strips};
use super::model::{
    MediaEntry, RawSnapshot, StoredTimeline, TIMELINE_FILES, json_rows, media_entries,
    parse_timeline_files, stored_timeline, timeline_files, timeline_folders,
};
use super::playable::{media_type, playable_timeline};
use crate::js;
use crate::projects::store::PROJECT_FILE;
use crate::resolve::connection::ResolveConnection;
use crate::rules::PROJECT_NOTES_FILE;

/// Snapshots live under this folder of the workspace, one subfolder per Resolve project.
pub const HISTORY_ROOT: &str = "resolve";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentTurn {
    pub provider: Provider,
    pub prompt: String,
    /// Resolve write calls, e.g. `create_timeline {"name":"Cut"}`.
    pub actions: Vec<String>,
    /// The agent's last reply.
    pub summary: String,
    /// The Linker-Edit thread the turn ran in.
    pub thread: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub commit: String,
    pub changes: Vec<String>,
}

/// A media file the browser can show or play, and its content type.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFile {
    pub path: String,
    pub r#type: &'static str,
}

/// The edit history of one project folder. GitHistory is the real one; app tests use a fake.
#[async_trait]
pub trait History: Send + Sync {
    /// Saves Resolve's open project. Without a turn the change is the user's own. None when nothing changed.
    async fn record(&self, turn: Option<AgentTurn>) -> anyhow::Result<Option<Saved>>;
    /// The last few saves, for an agent starting a new chat. Empty when there is no history yet.
    async fn recent_context(&self) -> anyhow::Result<String>;
    /// The newest saves, for the history panel; 50 when limit is None.
    async fn log(&self, limit: Option<usize>) -> anyhow::Result<Vec<HistoryEntry>>;
    /// A timeline the browser can play, as Resolve has it now (commit None) or as it was in a save.
    async fn timeline(
        &self,
        id: Option<String>,
        commit: Option<String>,
    ) -> anyhow::Result<Option<PlayableTimeline>>;
    /// A media file the browser can show or play; None for other formats and unknown ids.
    fn media_file(&self, id: &str) -> Option<MediaFile>;
    /// Commits the project's linker-edit.json when it changed, so the link travels with the history.
    async fn commit_project_file(&self, message: &str) -> anyhow::Result<()>;
}

/// The Resolve project a snapshot came from, for a ProjectCheck.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenSnapshotProject {
    pub name: String,
    pub id: Option<String>,
}

/// Says why Resolve's open project isn't the one this history belongs to, or None when it is. A save
/// refuses to write another project's timelines into this history.
pub type ProjectCheck = Arc<dyn Fn(&OpenSnapshotProject) -> Option<String> + Send + Sync>;

/// Which timelines a snapshot needs: all of them (no scope), or one by id, or (only None) the one Resolve shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotScope {
    pub only: Option<String>,
}

/// Reads Resolve's open project. The real one is ResolveSnapshots over the Resolve connection; tests give a
/// snapshot of their own.
#[async_trait]
pub trait SnapshotReader: Send + Sync {
    /// A save reads every timeline (scope None); the live preview asks for one only.
    async fn read_snapshot(&self, scope: Option<SnapshotScope>) -> anyhow::Result<RawSnapshot>;
}

/// What read_resolve_snapshot needs from the Resolve layer: running one of Linker-Edit's Python scripts in
/// Resolve (resolve/scripts/<script>.py) with its JSON arguments, and its JSON result. ResolveConnection.runScript
/// in the TypeScript.
#[async_trait]
pub trait ScriptRunner: Send + Sync {
    async fn run_script(
        &self,
        script: &str,
        args: Value,
        timeout_secs: u64,
    ) -> anyhow::Result<Value>;
}

#[async_trait]
impl ScriptRunner for dyn ResolveConnection {
    async fn run_script(
        &self,
        script: &str,
        args: Value,
        timeout_secs: u64,
    ) -> anyhow::Result<Value> {
        self.run_script_value(script, args, timeout_secs).await
    }
}

/// Resolve's open project with every timeline (or the one asked for), read in as many script runs as it takes.
pub async fn read_resolve_snapshot<R: ScriptRunner + ?Sized>(
    connection: &R,
    scope: Option<&SnapshotScope>,
) -> anyhow::Result<RawSnapshot> {
    fn page(mut value: Value) -> anyhow::Result<(RawSnapshot, Vec<String>)> {
        let remaining = match value
            .as_object_mut()
            .and_then(|page| page.remove("remaining"))
        {
            Some(remaining) => serde_json::from_value(remaining)?,
            None => Vec::new(),
        };
        Ok((serde_json::from_value(value)?, remaining))
    }
    let ids = scope
        .and_then(|scope| scope.only.clone())
        .map(|id| vec![id]);
    let current_only = scope.is_some_and(|scope| scope.only.is_none());
    // Scripts stop after 60 seconds, so each run stops after 40 and says which timelines are left.
    let first = connection
        .run_script(
            "snapshot",
            json!({ "ids": ids, "current_only": current_only, "budget": 40 }),
            60,
        )
        .await?;
    let (mut snapshot, mut remaining) = page(first)?;
    while !remaining.is_empty() {
        let next = connection
            .run_script("snapshot", json!({ "ids": remaining, "budget": 40 }), 60)
            .await?;
        let (next, left) = page(next)?;
        if next.project_id != snapshot.project_id {
            bail!("Resolve switched projects while Linker-Edit was saving it.");
        }
        if next.timelines.is_empty() {
            break;
        }
        snapshot.timelines.extend(next.timelines);
        remaining = left;
    }
    Ok(snapshot)
}

/// A SnapshotReader over the Resolve connection, as the app's createHistory had it in index.ts.
pub struct ResolveSnapshots<R: ?Sized>(pub Arc<R>);

#[async_trait]
impl<R: ScriptRunner + ?Sized> SnapshotReader for ResolveSnapshots<R> {
    async fn read_snapshot(&self, scope: Option<SnapshotScope>) -> anyhow::Result<RawSnapshot> {
        read_resolve_snapshot(self.0.as_ref(), scope.as_ref()).await
    }
}

struct GitOutput {
    code: i32,
    stdout: String,
    stderr: String,
}

fn git_command(cwd: &Path, args: &[String]) -> Command {
    let mut command = Command::new("git");
    command
        .args(args)
        .current_dir(cwd)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    js::hide_window(&mut command);
    command
}

async fn git(cwd: &Path, args: &[String], input: Option<&str>) -> anyhow::Result<GitOutput> {
    let mut child = git_command(cwd, args).spawn().context("Couldn't run git")?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = input.unwrap_or_default().as_bytes().to_vec();
    // git can exit before it reads stdin. Its exit code reports any failure, so a broken pipe is no error.
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
    });
    let output = child.wait_with_output().await?;
    let _ = writer.await;
    Ok(GitOutput {
        code: output.status.code().unwrap_or(1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

async fn git_ok(cwd: &Path, args: &[String], input: Option<&str>) -> anyhow::Result<String> {
    let result = git(cwd, args, input).await?;
    if result.code != 0 {
        let detail = if result.stderr.trim().is_empty() {
            result.stdout.trim()
        } else {
            result.stderr.trim()
        };
        bail!("git {} failed: {detail}", args[0]);
    }
    Ok(result.stdout)
}

fn strings(args: &[&str]) -> Vec<String> {
    args.iter().map(|arg| arg.to_string()).collect()
}

fn agent_name(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "Claude",
        Provider::Codex => "Codex",
    }
}

fn provider_id(provider: Provider) -> &'static str {
    match provider {
        Provider::Claude => "claude",
        Provider::Codex => "codex",
    }
}

fn author(provider: Provider) -> String {
    format!(
        "--author={} <{}@linker-edit.local>",
        agent_name(provider),
        provider_id(provider)
    )
}

/// `create_timeline {"name":"Cut"}` for a Resolve call that writes; None for reads and other tools.
/// is_resolve_write is policy.ts's isResolveWrite, from the Resolve layer.
pub fn resolve_write_action(
    tool: Option<&str>,
    input: &Value,
    is_resolve_write: impl Fn(&str) -> bool,
) -> Option<String> {
    let tool = tool.filter(|tool| !tool.is_empty())?;
    if !is_resolve_write(tool) {
        return None;
    }
    let name = tool.split("__").last().unwrap_or(tool);
    let args = match input {
        Value::Object(_) | Value::Array(_) => js::stringify(input),
        _ => String::new(),
    };
    let line = if !args.is_empty() && args != "{}" {
        format!("{name} {args}")
    } else {
        name.to_string()
    };
    if js::length(&line) > 240 {
        Some(format!("{}…", js::slice(&line, 239)))
    } else {
        Some(line)
    }
}

/// Puts recent history ahead of the first message of a chat, so a fresh agent knows what changed.
pub fn with_history_context(context: &str, text: &str) -> String {
    if context.is_empty() {
        return text.to_string();
    }
    format!(
        "<edit_history>
Recent saves of the Resolve project, newest first, from the Git history in your working folder. Use git log and git show for details.
{context}
</edit_history>

{text}"
    )
}

static WHITESPACE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());

fn subject_of(prompt: &str) -> String {
    let line = WHITESPACE.replace_all(prompt, " ");
    let line = js::trim(&line);
    if js::length(line) > 72 {
        format!("{}…", js::slice(line, 71))
    } else {
        line.to_string()
    }
}

fn bullets(lines: &[String]) -> String {
    lines
        .iter()
        .map(|line| format!("- {line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn commit_message(changes: &[String], turn: Option<&AgentTurn>) -> String {
    let change_block = format!(
        "Changes:\n{}",
        if changes.is_empty() {
            "- timeline details only".to_string()
        } else {
            bullets(changes)
        }
    );
    let Some(turn) = turn else {
        return format!("Changes made in Resolve\n\n{change_block}\n");
    };
    let summary = if js::length(&turn.summary) > 1500 {
        format!("{}…", js::slice(&turn.summary, 1500))
    } else {
        turn.summary.clone()
    };
    let subject = subject_of(&turn.prompt);
    let mut parts = vec![
        if subject.is_empty() {
            "Agent changes".to_string()
        } else {
            subject
        },
        format!("Prompt: {}", turn.prompt),
        format!(
            "Resolve actions:\n{}",
            if turn.actions.is_empty() {
                "- none".to_string()
            } else {
                bullets(&turn.actions)
            }
        ),
        change_block,
    ];
    if !summary.is_empty() {
        parts.push(format!("Agent summary:\n{summary}"));
    }
    if let Some(thread) = turn.thread.as_deref().filter(|thread| !thread.is_empty()) {
        parts.push(format!("Thread: {thread}"));
    }
    format!("{}\n", parts.join("\n\n"))
}

/// A bulleted section of a commit message written by commit_message, without its "- none" placeholder.
fn section(body: &str, title: &str) -> Vec<String> {
    let pattern = Regex::new(&format!(
        r"(?:^|\n){}:\n((?:- .*(?:\n|\z))*)",
        regex::escape(title)
    ))
    .expect("a valid pattern");
    let lines = pattern
        .captures(body)
        .and_then(|captures| captures.get(1))
        .map_or("", |found| found.as_str());
    lines
        .split('\n')
        .filter(|line| line.starts_with("- ") && *line != "- none")
        .map(|line| line[2..].to_string())
        .collect()
}

static PROMPT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?ms)^Prompt: (.*?)\n\nResolve actions:").unwrap());
static THREAD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n\nThread: (\S+)\s*\z").unwrap());

fn parse_entry(fields: &[&str]) -> HistoryEntry {
    let field = |index: usize| fields.get(index).copied().unwrap_or_default();
    let (commit, date, author, email, body) = (field(0), field(1), field(2), field(3), field(4));
    let agent = [Provider::Claude, Provider::Codex]
        .into_iter()
        .find(|provider| email == format!("{}@linker-edit.local", provider_id(*provider)));
    let capture = |pattern: &Regex| {
        pattern
            .captures(body)
            .and_then(|captures| captures.get(1))
            .map(|found| found.as_str().to_string())
    };
    HistoryEntry {
        commit: commit.into(),
        date: date.into(),
        author: author.into(),
        agent,
        subject: body.split('\n').next().unwrap_or_default().into(),
        prompt: capture(&PROMPT),
        actions: section(body, "Resolve actions"),
        changes: section(body, "Changes"),
        thread: capture(&THREAD),
        timelines: Vec::new(),
    }
}

/// Contents of each `<rev>:<path>`, or None where it doesn't exist (a new timeline has no `<commit>^` side).
async fn cat_files(cwd: &Path, names: &[String]) -> anyhow::Result<Vec<Option<String>>> {
    if names.is_empty() {
        return Ok(Vec::new());
    }
    let mut child = git_command(cwd, &strings(&["cat-file", "--batch"]))
        .spawn()
        .context("Couldn't run git")?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let input = format!("{}\n", names.join("\n")).into_bytes();
    let writer = tokio::spawn(async move {
        let _ = stdin.write_all(&input).await;
    });
    let output = child.wait_with_output().await?;
    let _ = writer.await;
    let out = output.stdout;
    let hex = |text: &str| {
        !text.is_empty()
            && text
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    };
    let mut at = 0;
    Ok(names
        .iter()
        .map(|_| {
            let line_end = out[at.min(out.len())..].iter().position(|&b| b == b'\n')? + at;
            let header = String::from_utf8_lossy(&out[at..line_end]).into_owned();
            let header: Vec<&str> = header.split(' ').collect();
            at = line_end + 1;
            if header.len() != 3 || header[1] != "blob" || !hex(header[0]) {
                return None;
            }
            let size: usize = header[2].parse().ok()?;
            let end = (at + size).min(out.len());
            let text = String::from_utf8_lossy(&out[at.min(end)..end]).into_owned();
            at += size + 1;
            Some(text)
        })
        .collect())
}

/// A timeline as it was in a save, with that save's media list.
#[derive(Debug, Clone, PartialEq)]
pub struct SavedTimeline {
    pub timeline: StoredTimeline,
    pub media: Vec<MediaEntry>,
}

static COMMIT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^[0-9a-f]{4,64}$").unwrap());

/// A timeline as it was in a save of the history in dir, found by its id or (by_name) its exact name, with
/// that save's media list. None when the save or the timeline isn't there. The TypeScript's byName defaulted to true.
pub async fn read_saved_timeline(
    dir: &Path,
    commit: &str,
    reference: &str,
    by_name: bool,
) -> anyhow::Result<Option<SavedTimeline>> {
    // The commit can come from a URL or an agent; a value starting with "-" would be read as a git option.
    if !COMMIT.is_match(commit) {
        return Ok(None);
    }
    let tree = git(
        dir,
        &strings(&[
            "-c",
            "core.quotePath=false",
            "ls-tree",
            "-r",
            "--name-only",
            commit,
            "--",
            HISTORY_ROOT,
        ]),
        None,
    )
    .await?;
    if tree.code != 0 {
        return Ok(None);
    }
    let metadata_files: Vec<&str> = tree
        .stdout
        .split('\n')
        .filter(|file| file.split('/').count() == 4 && file.ends_with("/metadata.json"))
        .collect();
    let names: Vec<String> = metadata_files
        .iter()
        .map(|file| format!("{commit}:{file}"))
        .collect();
    let metadata: Vec<Option<Value>> = cat_files(dir, &names)
        .await?
        .into_iter()
        .map(|text| text.and_then(|text| serde_json::from_str(&text).ok()))
        .collect();
    let field = |info: &Option<Value>, key: &str| {
        info.as_ref()
            .and_then(|info| info.get(key))
            .and_then(Value::as_str)
            .map(str::to_string)
    };
    let mut index = metadata
        .iter()
        .position(|info| field(info, "id").as_deref() == Some(reference));
    if index.is_none() && by_name {
        index = metadata
            .iter()
            .position(|info| field(info, "name").as_deref() == Some(reference));
    }
    let Some(index) = index else { return Ok(None) };
    let folder = metadata_files[index]
        .rsplit_once('/')
        .map_or("", |(folder, _)| folder);
    let project = folder.rsplit_once('/').map_or("", |(project, _)| project);
    let mut names: Vec<String> = TIMELINE_FILES
        .iter()
        .map(|file| format!("{commit}:{folder}/{file}"))
        .collect();
    names.push(format!("{commit}:{project}/media.json"));
    let files = cat_files(dir, &names).await?;
    let Some(timeline) = parse_timeline_files(|file| {
        TIMELINE_FILES
            .iter()
            .position(|candidate| *candidate == file)
            .and_then(|index| files[index].clone())
    })?
    else {
        return Ok(None);
    };
    #[derive(Deserialize)]
    struct MediaFile {
        media: Vec<MediaEntry>,
    }
    // Without that save's media.json, today's media list still finds most files.
    let media = files
        .last()
        .cloned()
        .flatten()
        .and_then(|text| serde_json::from_str::<MediaFile>(&text).ok())
        .map(|file| file.media)
        .unwrap_or_default();
    Ok(Some(SavedTimeline { timeline, media }))
}

#[derive(Deserialize)]
struct MediaList {
    media: Vec<MediaEntry>,
}

/// Media pool item id → file path, from the media.json of every project in the history.
fn read_media(dir: &Path) -> HashMap<String, MediaEntry> {
    let mut media = HashMap::new();
    let Ok(projects) = std::fs::read_dir(dir.join(HISTORY_ROOT)) else {
        return media;
    };
    for project in projects.flatten() {
        let file = project.path().join("media.json");
        if !project.file_type().is_ok_and(|kind| kind.is_dir()) || !file.exists() {
            continue;
        }
        // A hand-edited or half-merged media.json only costs thumbnails.
        let Ok(text) = std::fs::read_to_string(&file) else {
            continue;
        };
        let Ok(list) = serde_json::from_str::<MediaList>(&text) else {
            continue;
        };
        for entry in list.media {
            media.insert(entry.id.clone(), entry);
        }
    }
    media
}

fn read_stored(project_dir: &Path) -> anyhow::Result<Vec<StoredTimeline>> {
    if !project_dir.exists() {
        return Ok(Vec::new());
    }
    let mut timelines = Vec::new();
    for entry in std::fs::read_dir(project_dir)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let folder = entry.path();
        let timeline = parse_timeline_files(|file| {
            let full = folder.join(file);
            if full.exists() {
                std::fs::read_to_string(full).ok()
            } else {
                None
            }
        })
        .with_context(|| format!("Couldn't read the saved timeline in {}", folder.display()))?;
        timelines.extend(timeline);
    }
    Ok(timelines)
}

/// Says which Resolve project a history folder holds, so the folder can follow a rename in Resolve.
const PROJECT_INFO: &str = "project.json";

fn read_project_id(folder: &Path) -> Option<String> {
    let text = std::fs::read_to_string(folder.join(PROJECT_INFO)).ok()?;
    let info: Value = serde_json::from_str(&text).ok()?;
    info.get("id")?.as_str().map(str::to_string)
}

/// The folder a snapshot of this Resolve project goes in, named after the project, and the folder it was renamed
/// from. When Resolve renamed the project, its folder is renamed too, so its history goes on instead of starting
/// over in a new folder.
fn project_folder(
    dir: &Path,
    snapshot: &RawSnapshot,
) -> anyhow::Result<(PathBuf, Option<PathBuf>)> {
    let root = dir.join(HISTORY_ROOT);
    let folder = root.join(&timeline_folders(&[&snapshot.project])[0]);
    let Some(id) = snapshot.project_id.as_deref().filter(|id| !id.is_empty()) else {
        return Ok((folder, None));
    };
    if read_project_id(&folder).as_deref() == Some(id) || !root.exists() {
        return Ok((folder, None));
    }
    for entry in std::fs::read_dir(&root)? {
        let entry = entry?;
        let old = root.join(entry.file_name());
        if !entry.file_type()?.is_dir()
            || old == folder
            || read_project_id(&old).as_deref() != Some(id)
        {
            continue;
        }
        // Another folder already has the new name: keep saving where this project's history is.
        if folder.exists() {
            return Ok((old, None));
        }
        std::fs::rename(&old, &folder)?;
        return Ok((folder, Some(old)));
    }
    Ok((folder, None))
}

fn write_stored(
    project_dir: &Path,
    snapshot: &RawSnapshot,
    timelines: &[StoredTimeline],
) -> anyhow::Result<()> {
    let names: Vec<&str> = timelines
        .iter()
        .map(|timeline| timeline.metadata.name.as_str())
        .collect();
    let folders = timeline_folders(&names);
    std::fs::create_dir_all(project_dir)?;
    for entry in std::fs::read_dir(project_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if entry.file_type()?.is_dir() && !folders.contains(&name) {
            std::fs::remove_dir_all(entry.path())?;
        }
    }
    for (timeline, folder) in timelines.iter().zip(&folders) {
        let folder = project_dir.join(folder);
        std::fs::create_dir_all(&folder)?;
        for (file, contents) in timeline_files(timeline) {
            std::fs::write(folder.join(file), contents)?;
        }
    }
    std::fs::write(
        project_dir.join("media.json"),
        json_rows("media", &media_entries(snapshot)),
    )?;
    if let Some(id) = snapshot.project_id.as_deref().filter(|id| !id.is_empty()) {
        std::fs::write(
            project_dir.join(PROJECT_INFO),
            format!(
                "{}\n",
                js::stringify_pretty(&json!({ "id": id, "name": snapshot.project }))
            ),
        )?;
    }
    Ok(())
}

/// What a GitHistory is made with.
pub struct HistoryOptions {
    /// The project's folder; the repository is made there on first use.
    pub dir: PathBuf,
    /// A save reads every timeline; the live preview asks for one only.
    pub read_snapshot: Arc<dyn SnapshotReader>,
    pub check_project: Option<ProjectCheck>,
}

#[derive(Default)]
struct MediaCache {
    /// Media seen in live snapshots, so the quick preview finds media no save has recorded yet.
    snapshot: HashMap<String, MediaEntry>,
    /// The media lists of the saved projects, read once and again after each save: every thumbnail asks.
    saved: Option<HashMap<String, MediaEntry>>,
}

/// The edit history of a project folder, kept with the git CLI.
pub struct GitHistory {
    dir: PathBuf,
    read_snapshot: Arc<dyn SnapshotReader>,
    check_project: Option<ProjectCheck>,
    /// `-c user.name=… -c user.email=…` when git has no user name, worked out once.
    identity: OnceCell<Vec<String>>,
    // Saves run one at a time, so a turn's "before" save always lands before its "after" save.
    queue: tokio::sync::Mutex<()>,
    media: Mutex<MediaCache>,
}

pub fn create_history(options: HistoryOptions) -> GitHistory {
    GitHistory {
        dir: options.dir,
        read_snapshot: options.read_snapshot,
        check_project: options.check_project,
        identity: OnceCell::new(),
        queue: tokio::sync::Mutex::new(()),
        media: Mutex::new(MediaCache::default()),
    }
}

fn user_name() -> String {
    let name = std::env::var(if cfg!(windows) { "USERNAME" } else { "USER" })
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default();
    if name.is_empty() {
        "Linker-Edit user".into()
    } else {
        name
    }
}

impl GitHistory {
    fn known_media(&self) -> HashMap<String, MediaEntry> {
        let mut cache = self.media.lock().unwrap();
        cache
            .saved
            .get_or_insert_with(|| read_media(&self.dir))
            .clone()
    }

    async fn prepare(&self) -> anyhow::Result<Vec<String>> {
        let identity = self
            .identity
            .get_or_try_init(|| async {
                std::fs::create_dir_all(&self.dir)?;
                let inside = git(
                    &self.dir,
                    &strings(&["rev-parse", "--is-inside-work-tree"]),
                    None,
                )
                .await?;
                if inside.code != 0 {
                    git_ok(&self.dir, &strings(&["init", "-q"]), None).await?;
                }
                let name = git(&self.dir, &strings(&["config", "user.name"]), None).await?;
                if !name.stdout.trim().is_empty() {
                    return Ok::<_, anyhow::Error>(Vec::new());
                }
                let user = user_name();
                Ok(vec![
                    "-c".into(),
                    format!("user.name={user}"),
                    "-c".into(),
                    format!("user.email={user}@linker-edit.local"),
                ])
            })
            .await?;
        Ok(identity.clone())
    }

    async fn save(&self, turn: Option<&AgentTurn>) -> anyhow::Result<Option<Saved>> {
        let fallback_identity = self.prepare().await?;
        let snapshot = self.read_snapshot.read_snapshot(None).await?;
        if let Some(check) = &self.check_project {
            let open = OpenSnapshotProject {
                name: snapshot.project.clone(),
                id: snapshot.project_id.clone(),
            };
            if let Some(refusal) = check(&open) {
                return Err(anyhow!(refusal));
            }
        }
        let (project_dir, renamed_from) = project_folder(&self.dir, &snapshot)?;
        let before = read_stored(&project_dir)?;
        let after: Vec<StoredTimeline> = snapshot.timelines.iter().map(stored_timeline).collect();
        if before.is_empty() && after.is_empty() && renamed_from.is_none() {
            return Ok(None);
        }
        // Histories from before project.json get it with their next save.
        let noted_id = !before.is_empty()
            && snapshot
                .project_id
                .as_deref()
                .is_some_and(|id| !id.is_empty())
            && read_project_id(&project_dir) != snapshot.project_id;
        write_stored(&project_dir, &snapshot, &after)?;
        self.media.lock().unwrap().saved = None;

        let relative = |folder: &Path| {
            folder
                .strip_prefix(&self.dir)
                .unwrap_or(folder)
                .to_string_lossy()
                .replace('\\', "/")
        };
        let mut pathspec = vec![relative(&project_dir)];
        pathspec.extend(renamed_from.as_deref().map(relative));
        let with_pathspec = |args: &[&str]| {
            let mut all = strings(args);
            all.push("--".into());
            all.extend(pathspec.iter().cloned());
            all
        };
        git_ok(&self.dir, &with_pathspec(&["add", "-A"]), None).await?;
        if git(
            &self.dir,
            &with_pathspec(&["diff", "--cached", "--quiet"]),
            None,
        )
        .await?
        .code
            == 0
        {
            return Ok(None);
        }

        let mut changes = Vec::new();
        if let Some(old) = &renamed_from {
            let old_name = old.file_name().unwrap_or_default().to_string_lossy();
            changes.push(format!(
                "Resolve project renamed from \"{old_name}\" to \"{}\"",
                snapshot.project
            ));
        }
        changes.extend(describe_changes(&before, &after));
        if changes.is_empty() && noted_id {
            changes.push(
                "Noted the Resolve project's id, so renaming it in Resolve keeps this history"
                    .into(),
            );
        }
        let mut args = fallback_identity;
        args.extend(strings(&["commit", "-q", "--no-verify"]));
        args.extend(turn.map(|turn| author(turn.provider)));
        args.extend(with_pathspec(&["-F", "-"]));
        git_ok(&self.dir, &args, Some(&commit_message(&changes, turn))).await?;
        let commit = git_ok(&self.dir, &strings(&["rev-parse", "--short", "HEAD"]), None)
            .await?
            .trim()
            .to_string();
        Ok(Some(Saved { commit, changes }))
    }

    async fn playable(
        &self,
        id: Option<String>,
        commit: Option<String>,
    ) -> anyhow::Result<Option<PlayableTimeline>> {
        self.prepare().await?;
        let Some(commit) = commit.filter(|commit| !commit.is_empty()) else {
            let snapshot = self
                .read_snapshot
                .read_snapshot(Some(SnapshotScope { only: id.clone() }))
                .await?;
            let media = {
                let mut cache = self.media.lock().unwrap();
                for entry in media_entries(&snapshot) {
                    cache.snapshot.insert(entry.id.clone(), entry);
                }
                cache.snapshot.clone()
            };
            let raw = snapshot.timelines.iter().find(|candidate| match &id {
                Some(id) if !id.is_empty() => &candidate.id == id,
                _ => snapshot.current_timeline.as_ref() == Some(&candidate.name),
            });
            return Ok(raw.map(|raw| playable_timeline(&stored_timeline(raw), &media, None)));
        };
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            return Ok(None);
        };
        let Some(saved) = read_saved_timeline(&self.dir, &commit, &id, false).await? else {
            return Ok(None);
        };
        let mut media = self.known_media();
        for entry in saved.media {
            media.insert(entry.id.clone(), entry);
        }
        Ok(Some(playable_timeline(
            &saved.timeline,
            &media,
            Some(commit),
        )))
    }

    async fn read_log(&self, limit: usize) -> anyhow::Result<Vec<HistoryEntry>> {
        self.prepare().await?;
        let mut args = strings(&["-c", "core.quotePath=false", "log", "-n"]);
        args.push(limit.to_string());
        args.extend(strings(&[
            "--raw",
            "--no-renames",
            "--format=%x1e%H%x1f%h%x1f%aI%x1f%an%x1f%ae%x1f%B%x1f",
            "--",
            HISTORY_ROOT,
        ]));
        let result = git(&self.dir, &args, None).await?;
        if result.code != 0 {
            return Ok(Vec::new());
        }
        struct Commit {
            hash: String,
            folders: Vec<String>,
            entry: HistoryEntry,
        }
        let commits: Vec<Commit> = result
            .stdout
            .split('\u{1e}')
            .filter(|entry| !js::trim(entry).is_empty())
            .map(|entry| {
                let mut fields: Vec<&str> = entry.split('\u{1f}').collect();
                let hash = if fields.is_empty() {
                    ""
                } else {
                    fields.remove(0)
                };
                let raw = fields.pop().unwrap_or_default();
                // --raw lines end in a tab and the path; a timeline's files sit at resolve/<project>/<timeline>/<file>.
                let mut folders: Vec<String> = Vec::new();
                for line in raw.split('\n') {
                    let Some(path) = line.split('\t').nth(1) else {
                        continue;
                    };
                    let parts: Vec<&str> = path.split('/').collect();
                    if parts.len() == 4 {
                        let folder = parts[..3].join("/");
                        if !folders.contains(&folder) {
                            folders.push(folder);
                        }
                    }
                }
                Commit {
                    hash: hash.to_string(),
                    folders,
                    entry: parse_entry(&fields),
                }
            })
            .collect();

        // Each touched timeline before and after its commit, read in one git process.
        let names: Vec<String> = commits
            .iter()
            .flat_map(|commit| {
                commit.folders.iter().flat_map(move |folder| {
                    TIMELINE_FILES.iter().flat_map(move |file| {
                        [
                            format!("{}^:{folder}/{file}", commit.hash),
                            format!("{}:{folder}/{file}", commit.hash),
                        ]
                    })
                })
            })
            .collect();
        let blobs: HashMap<String, Option<String>> = names
            .iter()
            .cloned()
            .zip(cat_files(&self.dir, &names).await?)
            .collect();
        let media = self.known_media();
        let is_image = |id: &str| {
            media_type(media.get(id).map(|entry| entry.path.as_str()))
                .is_some_and(|found| found.kind == MediaKind::Image)
        };
        Ok(commits
            .into_iter()
            .map(|commit| {
                let side = |rev: &str| -> Vec<StoredTimeline> {
                    commit
                        .folders
                        .iter()
                        .filter_map(|folder| {
                            parse_timeline_files(|file| {
                                blobs
                                    .get(&format!("{rev}:{folder}/{file}"))
                                    .cloned()
                                    .flatten()
                            })
                            .ok()
                            .flatten()
                        })
                        .collect()
                };
                let timelines = timeline_strips(
                    &side(&format!("{}^", commit.hash)),
                    &side(&commit.hash),
                    is_image,
                );
                HistoryEntry {
                    timelines,
                    ..commit.entry
                }
            })
            .collect())
    }

    async fn context(&self) -> anyhow::Result<String> {
        Ok(self
            .read_log(5)
            .await?
            .into_iter()
            .map(|entry| {
                // %aI is local time with an offset, so its first 16 characters read as the user's clock.
                let when = js::slice(&entry.date, 16).replacen('T', " ", 1);
                let details = if entry.changes.is_empty() {
                    String::new()
                } else {
                    let lines: Vec<String> = bullets(&entry.changes)
                        .split('\n')
                        .map(|line| format!("  {line}"))
                        .collect();
                    format!("\n{}", lines.join("\n"))
                };
                format!(
                    "{} {when} {}: {}{details}",
                    entry.commit, entry.author, entry.subject
                )
            })
            .collect::<Vec<_>>()
            .join("\n"))
    }

    /// Commits one file of the project's folder when it changed; as the agent when given.
    async fn commit_file(
        &self,
        file: &str,
        message: &str,
        provider: Option<Provider>,
    ) -> anyhow::Result<()> {
        let fallback_identity = self.prepare().await?;
        if !self.dir.join(file).exists() {
            return Ok(());
        }
        git_ok(&self.dir, &strings(&["add", "--", file]), None).await?;
        if git(
            &self.dir,
            &strings(&["diff", "--cached", "--quiet", "--", file]),
            None,
        )
        .await?
        .code
            == 0
        {
            return Ok(());
        }
        let mut args = fallback_identity;
        args.extend(strings(&["commit", "-q", "--no-verify"]));
        args.extend(provider.map(author));
        args.extend(strings(&["-F", "-", "--", file]));
        git_ok(&self.dir, &args, Some(&format!("{message}\n"))).await?;
        Ok(())
    }

    /// A save of Resolve, then the project's own file when it changed. Also when the save fails, so the file
    /// isn't lost with it. Without a turn it may be the user's edit, or an agent's from a turn that left Resolve alone.
    async fn save_with_notes(&self, turn: Option<&AgentTurn>) -> anyhow::Result<Option<Saved>> {
        let saved = self.save(turn).await;
        let message = match turn {
            Some(turn) => format!("Update {PROJECT_NOTES_FILE}: {}", subject_of(&turn.prompt)),
            None => format!("Update {PROJECT_NOTES_FILE}"),
        };
        if let Err(error) = self
            .commit_file(PROJECT_NOTES_FILE, &message, turn.map(|turn| turn.provider))
            .await
        {
            tracing::error!("Couldn't save {PROJECT_NOTES_FILE} to the edit history: {error:#}");
        }
        saved
    }
}

#[async_trait]
impl History for GitHistory {
    async fn record(&self, turn: Option<AgentTurn>) -> anyhow::Result<Option<Saved>> {
        let _queue = self.queue.lock().await;
        self.save_with_notes(turn.as_ref()).await
    }

    async fn recent_context(&self) -> anyhow::Result<String> {
        let _queue = self.queue.lock().await;
        self.context().await
    }

    async fn log(&self, limit: Option<usize>) -> anyhow::Result<Vec<HistoryEntry>> {
        let _queue = self.queue.lock().await;
        self.read_log(limit.unwrap_or(50)).await
    }

    async fn timeline(
        &self,
        id: Option<String>,
        commit: Option<String>,
    ) -> anyhow::Result<Option<PlayableTimeline>> {
        let _queue = self.queue.lock().await;
        self.playable(id, commit).await
    }

    fn media_file(&self, id: &str) -> Option<MediaFile> {
        let snapshot = self.media.lock().unwrap().snapshot.get(id).cloned();
        let file = snapshot.or_else(|| self.known_media().remove(id))?.path;
        let found = media_type(Some(&file))?;
        Path::new(&file).exists().then_some(MediaFile {
            path: file,
            r#type: found.r#type,
        })
    }

    async fn commit_project_file(&self, message: &str) -> anyhow::Result<()> {
        let _queue = self.queue.lock().await;
        self.commit_file(PROJECT_FILE, message, None).await
    }
}

#[cfg(test)]
mod tests {
    use std::process::Command as StdCommand;

    use serde_json::json;

    use super::*;

    /// A Resolve whose open project the test sets.
    struct FakeResolve(Mutex<RawSnapshot>);

    #[async_trait]
    impl SnapshotReader for FakeResolve {
        async fn read_snapshot(
            &self,
            _scope: Option<SnapshotScope>,
        ) -> anyhow::Result<RawSnapshot> {
            Ok(self.0.lock().unwrap().clone())
        }
    }

    fn item(id: &str, start: u32, properties: Value) -> Value {
        let mut all = json!({ "ZoomX": 1, "Opacity": 100 });
        all.as_object_mut()
            .unwrap()
            .extend(properties.as_object().unwrap().clone());
        json!({
            "id": id,
            "name": format!("clip {id}"),
            "start": start,
            "end": start + 48,
            "source_start": 0,
            "source_end": 48,
            "media": { "id": format!("m-{id}"), "name": format!("{id}.mov"), "path": format!("D:\\footage\\{id}.mov") },
            "enabled": true,
            "properties": all,
        })
    }

    fn raw_timeline(id: &str, name: &str, items: Vec<Value>) -> Value {
        json!({
            "id": id,
            "name": name,
            "frame_rate": "24",
            "width": "1920",
            "height": "1080",
            "start_frame": 86400,
            "end_frame": 86400 + items.len() * 48,
            "start_timecode": "01:00:00:00",
            "video": [{ "index": 1, "name": "Video 1", "items": items }],
            "audio": [{ "index": 1, "name": "Audio 1", "items": [] }],
            "markers": [],
        })
    }

    fn snapshot(value: Value) -> RawSnapshot {
        serde_json::from_value(value).unwrap()
    }

    fn run_git(dir: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8(output.stdout).unwrap()
    }

    struct Setup {
        _temp: tempfile::TempDir,
        dir: PathBuf,
        history: GitHistory,
        resolve: Arc<FakeResolve>,
    }

    impl Setup {
        fn new(start: Value) -> Self {
            let temp = tempfile::tempdir().unwrap();
            let dir = temp.path().to_path_buf();
            let resolve = Arc::new(FakeResolve(Mutex::new(snapshot(start))));
            let history = create_history(HistoryOptions {
                dir: dir.clone(),
                read_snapshot: resolve.clone(),
                check_project: None,
            });
            Setup {
                _temp: temp,
                dir,
                history,
                resolve,
            }
        }

        fn show(&self, timelines: Vec<Value>) {
            let mut snapshot = self.resolve.0.lock().unwrap();
            snapshot.timelines = timelines
                .into_iter()
                .map(|timeline| serde_json::from_value(timeline).unwrap())
                .collect();
        }

        fn read(&self, file: &str) -> String {
            std::fs::read_to_string(self.dir.join("resolve").join("Doc_ Final_").join(file))
                .unwrap()
        }

        fn git(&self, args: &[&str]) -> String {
            run_git(&self.dir, args)
        }
    }

    fn doc() -> Setup {
        Setup::new(json!({ "project": "Doc: Final?", "current_timeline": null, "timelines": [] }))
    }

    #[tokio::test]
    async fn commits_a_snapshot_only_when_resolve_changed_attributing_agent_turns_to_the_agent() {
        let setup = doc();
        let history = &setup.history;
        setup.show(vec![raw_timeline(
            "t1",
            "Interview",
            vec![
                item("a", 86400, json!({})),
                item("b", 86448, json!({ "ZoomX": 1.5 })),
            ],
        )]);

        let saved = history.record(None).await.unwrap().unwrap();
        assert_eq!(
            saved.changes,
            ["\"Interview\": new timeline, 2 clips, 0:04"]
        );
        assert_eq!(history.record(None).await.unwrap(), None);
        assert_eq!(
            setup.read("Interview/cuts.json"),
            [
                r#"{ "items": ["#,
                r#"  {"track":1,"id":"a","name":"clip a","start":86400,"end":86448,"source_start":0,"source_end":48,"media":"m-a"},"#,
                r#"  {"track":1,"id":"b","name":"clip b","start":86448,"end":86496,"source_start":0,"source_end":48,"media":"m-b","properties":{"ZoomX":1.5}}"#,
                "] }",
                "",
            ]
            .join("\n")
        );
        let media: Value = serde_json::from_str(&setup.read("media.json")).unwrap();
        assert_eq!(media["media"].as_array().unwrap().len(), 2);

        setup.show(vec![
            raw_timeline(
                "t1",
                "Interview",
                vec![
                    item("a", 86400, json!({})),
                    item("b", 86448, json!({ "ZoomX": 1.5 })),
                ],
            ),
            raw_timeline("t2", "AI - Tight", vec![item("b", 86400, json!({}))]),
        ]);
        let saved = history
            .record(Some(AgentTurn {
                provider: Provider::Claude,
                prompt: "Make a tighter cut\nwith only clip b".into(),
                actions: vec![r#"timeline.create_empty {"name":"AI - Tight"}"#.into()],
                summary: "Built AI - Tight with clip b.".into(),
                thread: Some("thread-1".into()),
            }))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.changes,
            ["\"AI - Tight\": new timeline, 1 clip, 0:02"]
        );
        assert_eq!(
            setup.git(&["log", "-1", "--format=%an%n%B"]),
            [
                "Claude",
                "Make a tighter cut with only clip b",
                "",
                "Prompt: Make a tighter cut",
                "with only clip b",
                "",
                "Resolve actions:",
                r#"- timeline.create_empty {"name":"AI - Tight"}"#,
                "",
                "Changes:",
                r#"- "AI - Tight": new timeline, 1 clip, 0:02"#,
                "",
                "Agent summary:",
                "Built AI - Tight with clip b.",
                "",
                "Thread: thread-1",
                "",
                "",
            ]
            .join("\n")
        );

        let tight_save = history.log(Some(1)).await.unwrap()[0].commit.clone();

        setup.show(vec![raw_timeline(
            "t1",
            "Interview",
            vec![
                item("a", 86400, json!({})),
                item("b", 86448, json!({ "ZoomX": 1.5 })),
            ],
        )]);
        let saved = history.record(None).await.unwrap().unwrap();
        assert_eq!(saved.changes, ["\"AI - Tight\": timeline removed"]);
        // A removed timeline still plays from the save that had it; the live one comes from Resolve.
        let played = history
            .timeline(Some("t2".into()), Some(tight_save.clone()))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(played.name, "AI - Tight");
        assert_eq!(played.commit.as_deref(), Some(tight_save.as_str()));
        assert_eq!(played.duration, 2.0);
        assert_eq!(played.video.len(), 1);
        let clip = &played.video[0];
        assert_eq!(
            (
                clip.name.as_str(),
                clip.start,
                clip.end,
                clip.media.as_deref()
            ),
            ("clip b", 0.0, 2.0, Some("m-b"))
        );
        assert_eq!(
            history
                .timeline(Some("t2".into()), Some("--output=x".into()))
                .await
                .unwrap(),
            None
        );
        assert_eq!(
            history.timeline(Some("t2".into()), None).await.unwrap(),
            None
        );
        assert_eq!(history.timeline(None, None).await.unwrap(), None);
        let live = history
            .timeline(Some("t1".into()), None)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(live.commit, None);
        let names: Vec<&str> = live.video.iter().map(|clip| clip.name.as_str()).collect();
        assert_eq!(names, ["clip a", "clip b"]);
        assert_eq!(
            serde_json::to_value(&live.video[1].look).unwrap(),
            json!({ "ZoomX": 1.5 })
        );
        assert_eq!(history.media_file("m-a"), None);
        assert!(
            !setup
                .dir
                .join("resolve")
                .join("Doc_ Final_")
                .join("AI_Tight")
                .exists()
        );
        assert_eq!(setup.git(&["status", "--porcelain"]), "");

        let dates = Regex::new(r" \d{4}-\d\d-\d\d \d\d:\d\d ").unwrap();
        let context = history.recent_context().await.unwrap();
        let context = dates.replace_all(&context, " DATE ");
        let expected = Regex::new(
            &[
                r"^\w+ DATE .+: Changes made in Resolve",
                r#"  - "AI - Tight": timeline removed"#,
                r"\w+ DATE Claude: Make a tighter cut with only clip b",
                r#"  - "AI - Tight": new timeline, 1 clip, 0:02"#,
                r"\w+ DATE .+: Changes made in Resolve",
                r#"  - "Interview": new timeline, 2 clips, 0:04$"#,
            ]
            .join("\n"),
        )
        .unwrap();
        assert!(expected.is_match(&context), "{context}");

        let log = history.log(None).await.unwrap();
        assert_eq!(log.len(), 3);
        assert_eq!(
            (
                log[0].agent,
                log[0].subject.as_str(),
                &log[0].prompt,
                &log[0].thread
            ),
            (None, "Changes made in Resolve", &None, &None)
        );
        assert!(log[0].actions.is_empty());
        assert_eq!(log[0].changes, ["\"AI - Tight\": timeline removed"]);
        assert!(log[0].timelines.is_empty());

        let agent = &log[1];
        assert_eq!(agent.author, "Claude");
        assert_eq!(agent.agent, Some(Provider::Claude));
        assert_eq!(agent.subject, "Make a tighter cut with only clip b");
        assert_eq!(
            agent.prompt.as_deref(),
            Some("Make a tighter cut\nwith only clip b")
        );
        assert_eq!(
            agent.actions,
            [r#"timeline.create_empty {"name":"AI - Tight"}"#]
        );
        assert_eq!(
            agent.changes,
            ["\"AI - Tight\": new timeline, 1 clip, 0:02"]
        );
        assert_eq!(agent.thread.as_deref(), Some("thread-1"));
        assert!(
            Regex::new(r"^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d")
                .unwrap()
                .is_match(&agent.date)
        );
        assert_eq!(
            serde_json::to_value(&agent.timelines).unwrap(),
            json!([{
                "id": "t2",
                "name": "AI - Tight",
                "state": "new",
                "tiles": [{ "kind": "clip", "name": "clip b", "media": "m-b", "image": false, "change": null }],
            }])
        );
        assert_eq!(log[2].agent, None);
        assert_eq!(
            log[2].changes,
            ["\"Interview\": new timeline, 2 clips, 0:04"]
        );
        assert_eq!(log[2].timelines.len(), 1);
        assert_eq!(log[2].timelines[0].name, "Interview");
        assert_eq!(
            log[2].timelines[0].state,
            linker_edit_protocol::TimelineStripState::New
        );
    }

    #[tokio::test]
    async fn refuses_to_save_another_resolve_project_and_commits_the_project_file() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().to_path_buf();
        let resolve = Arc::new(FakeResolve(Mutex::new(snapshot(json!({
            "project": "Promo",
            "project_id": "p-2",
            "current_timeline": null,
            "timelines": [raw_timeline("t1", "Cut", vec![item("a", 86400, json!({}))])],
        })))));
        let check: ProjectCheck = Arc::new(|open: &OpenSnapshotProject| {
            (open.id.as_deref() != Some("p-1"))
                .then(|| format!("Resolve has \"{}\" open.", open.name))
        });
        let history = create_history(HistoryOptions {
            dir: dir.clone(),
            read_snapshot: resolve,
            check_project: Some(check),
        });
        // The project's own file is saved once with a save, as the turn's agent, also when Resolve's side is refused.
        std::fs::write(dir.join("PROJECT.md"), "# Promo\n").unwrap();
        let turn = AgentTurn {
            provider: Provider::Claude,
            prompt: "Log the footage".into(),
            actions: vec![],
            summary: String::new(),
            thread: None,
        };
        let refused = history.record(Some(turn)).await.unwrap_err();
        assert_eq!(refused.to_string(), "Resolve has \"Promo\" open.");
        let refused = history.record(None).await.unwrap_err();
        assert_eq!(refused.to_string(), "Resolve has \"Promo\" open.");
        assert!(!dir.join("resolve").exists());

        std::fs::write(dir.join("linker-edit.json"), "{}\n").unwrap();
        history
            .commit_project_file("Link to Resolve project Doc")
            .await
            .unwrap();
        history
            .commit_project_file("Link to Resolve project Doc")
            .await
            .unwrap();
        let log = run_git(&dir, &["log", "--format=%an: %s"]);
        assert!(
            Regex::new(
                r"^.+: Link to Resolve project Doc\nClaude: Update PROJECT\.md: Log the footage\n$"
            )
            .unwrap()
            .is_match(&log),
            "{log}"
        );
    }

    #[tokio::test]
    async fn keeps_one_history_when_the_project_is_renamed_in_resolve() {
        let setup = Setup::new(json!({
            "project": "Promo",
            "project_id": "p-1",
            "current_timeline": null,
            "timelines": [raw_timeline("t1", "Cut", vec![item("a", 86400, json!({}))])],
        }));
        setup.history.record(None).await.unwrap();
        setup.resolve.0.lock().unwrap().project = "Promo v2".into();
        let saved = setup.history.record(None).await.unwrap().unwrap();
        assert_eq!(
            saved.changes,
            ["Resolve project renamed from \"Promo\" to \"Promo v2\""]
        );
        let folders: Vec<String> = std::fs::read_dir(setup.dir.join("resolve"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(folders, ["Promo v2"]);
        assert_eq!(setup.git(&["status", "--porcelain"]), "");
        // The timeline itself didn't change, so the history of its cuts goes on.
        let follow = setup.git(&[
            "log",
            "--format=%s",
            "--follow",
            "--",
            "resolve/Promo v2/Cut/cuts.json",
        ]);
        assert_eq!(follow.trim().split('\n').count(), 2);
        assert_eq!(setup.history.record(None).await.unwrap(), None);
    }

    #[tokio::test]
    async fn saves_nothing_for_a_project_without_timelines() {
        let setup = doc();
        assert_eq!(setup.history.record(None).await.unwrap(), None);
        assert_eq!(setup.history.recent_context().await.unwrap(), "");
    }

    /// Reads a snapshot in pages, as snapshot.py hands it over.
    struct PagedResolve(Mutex<Vec<(String, Value)>>);

    #[async_trait]
    impl ScriptRunner for PagedResolve {
        async fn run_script(
            &self,
            script: &str,
            args: Value,
            timeout_secs: u64,
        ) -> anyhow::Result<Value> {
            assert_eq!((script, timeout_secs), ("snapshot", 60));
            let mut pages = self.0.lock().unwrap();
            let (expected, page) = pages.remove(0);
            assert_eq!(args.to_string(), expected);
            Ok(page)
        }
    }

    #[tokio::test]
    async fn reads_a_long_project_in_pages() {
        let timeline = |id: &str| raw_timeline(id, id, vec![]);
        let pages = PagedResolve(Mutex::new(vec![
            (
                r#"{"ids":null,"current_only":false,"budget":40}"#.into(),
                json!({ "project": "Doc", "project_id": "p", "current_timeline": "a", "timelines": [timeline("a")], "remaining": ["b", "c"] }),
            ),
            (
                r#"{"ids":["b","c"],"budget":40}"#.into(),
                json!({ "project": "Doc", "project_id": "p", "current_timeline": "a", "timelines": [timeline("b")], "remaining": ["c"] }),
            ),
            (
                r#"{"ids":["c"],"budget":40}"#.into(),
                json!({ "project": "Doc", "project_id": "p", "current_timeline": "a", "timelines": [timeline("c")], "remaining": [] }),
            ),
        ]));
        let snapshot = read_resolve_snapshot(&pages, None).await.unwrap();
        let ids: Vec<&str> = snapshot
            .timelines
            .iter()
            .map(|timeline| timeline.id.as_str())
            .collect();
        assert_eq!(ids, ["a", "b", "c"]);
        assert_eq!(snapshot.current_timeline.as_deref(), Some("a"));
    }

    #[test]
    fn resolve_write_action_cases() {
        // A stand-in for policy.ts's isResolveWrite.
        let is_write = |tool: &str| {
            matches!(
                tool,
                "mcp__linker__create_timeline"
                    | "mcp__linker__add_markers"
                    | "mcp__resolve__run_script"
            )
        };
        let cases: [(Option<&str>, Value, Option<&str>); 9] = [
            (
                Some("mcp__linker__create_timeline"),
                json!({ "name": "Cut" }),
                Some(r#"create_timeline {"name":"Cut"}"#),
            ),
            (
                Some("mcp__linker__add_markers"),
                json!({}),
                Some("add_markers"),
            ),
            (
                Some("mcp__linker__get_timeline"),
                json!({ "timeline": "AI - Cut" }),
                None,
            ),
            (
                Some("mcp__linker__set_current_timeline"),
                json!({ "timeline": "AI - Cut" }),
                None,
            ),
            (
                Some("mcp__linker__transcribe"),
                json!({ "files": ["a.mov"] }),
                None,
            ),
            (
                Some("mcp__resolve__run_script"),
                json!({ "script": "x = 1" }),
                Some(r#"run_script {"script":"x = 1"}"#),
            ),
            (
                Some("mcp__resolve__search_scripting_api"),
                json!({ "pattern": "Marker" }),
                None,
            ),
            (Some("Write"), json!({ "file_path": "notes.md" }), None),
            (None, json!({}), None),
        ];
        for (tool, input, expected) in cases {
            assert_eq!(
                resolve_write_action(tool, &input, is_write).as_deref(),
                expected,
                "{tool:?}"
            );
        }
        let long = resolve_write_action(
            Some("mcp__resolve__run_script"),
            &json!({ "script": "x".repeat(300) }),
            is_write,
        )
        .unwrap();
        assert_eq!(js::length(&long), 240);
        assert!(long.ends_with('…'));
    }
}
