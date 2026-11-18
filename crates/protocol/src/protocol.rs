// Messages exchanged between the local server and the browser UI over /ws.
//
// apps/desktop/shared/protocol.ts is generated from these types (see typescript.rs), so the Svelte UI, the iPad
// app and this crate speak the same JSON. `field?: T` in TypeScript is an Option that's left out when None;
// `field: T | null` is an Option that's always sent, as null when None.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use ts_rs::TS;

use crate::number;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Claude,
    Codex,
}

/// Like T3 Code's runtime modes. "auto" (the default) is full access: everything allowed runs without asking.
/// "ask" is supervised: Resolve changes, scripts and file edits wait for the user. What the policy refuses,
/// it refuses in both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    Ask,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ToolStatus {
    Running,
    Done,
    Error,
}

/// How a notice or a toast reads: "info" | "error".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum Tone {
    Info,
    Error,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ChatItem {
    User {
        id: String,
        text: String,
    },
    Assistant {
        id: String,
        text: String,
    },
    Tool {
        id: String,
        name: String,
        #[serde(default)]
        #[ts(type = "unknown")]
        input: Value,
        status: ToolStatus,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        output: Option<String>,
    },
    Notice {
        id: String,
        text: String,
        #[ts(inline)]
        tone: Tone,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ResolveStatus {
    Checking,
    Connected {
        product: String,
        version: String,
        project: Option<String>,
        /// Resolve's unique id for the open project; null when none is open or Resolve doesn't say.
        project_id: Option<String>,
        /// The project library the open project lives in.
        database: Option<ResolveDatabase>,
        timeline: Option<String>,
    },
    NotRunning,
    Unreachable {
        hint: String,
    },
    Blocked {
        hint: String,
    },
    SetupNeeded {
        hint: String,
    },
}

/// A project library as Resolve's GetCurrentDatabase() reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResolveDatabase {
    /// "Disk" or "PostgreSQL" in Resolve's documentation; anything else is treated as a cloud library.
    pub r#type: String,
    pub name: String,
    /// PostgreSQL server address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub host: Option<String>,
}

/// Where a ResolveLink's project lives: "local" | "network" | "cloud".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ResolveLinkKind {
    Local,
    Network,
    Cloud,
}

/// Which Resolve project a Linker-Edit project belongs to. Only what is the same on every machine,
/// so it can be committed and shared; see LocalLink for the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResolveLink {
    #[ts(inline)]
    pub kind: ResolveLinkKind,
    pub name: String,
    /// Resolve's unique project id once Linker-Edit has seen the project open; null before.
    pub project_id: Option<String>,
}

/// This machine's side of a link: where to find the project, never committed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LocalLink {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub database: Option<ResolveDatabase>,
    /// Project Manager folders down to the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub folder: Option<Vec<String>>,
    /// The project's own library folders, read with the server's library::folders::project_folders. Not in
    /// protocol.ts: the TypeScript server kept them on this object as an extra key, and local.json still does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(skip)]
    pub library: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    /// ISO 8601.
    pub updated_at: String,
    pub archived: bool,
    pub busy: bool,
    /// An approval is waiting for the user.
    pub waiting: bool,
    #[serde(flatten)]
    pub sidebar: ThreadSidebarState,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    /// The project's working folder: its edit history, notes and exports.
    pub dir: String,
    /// Null until the project's first turn with Resolve open, or until it is linked by hand.
    pub link: Option<ResolveLink>,
    pub local: LocalLink,
    /// Its Resolve project is the one open in Resolve now.
    pub open: bool,
    /// Newest first, archived ones included.
    pub threads: Vec<ThreadSummary>,
    /// A workspace's or series' setup threads: the agent studies example videos and writes the brief here. It has no
    /// Resolve project and never gets one.
    pub setup: bool,
    #[serde(flatten)]
    pub sidebar: ProjectSidebarState,
}

/// A project in the Resolve database that is open now, for adding it to Linker-Edit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryProject {
    pub name: String,
    pub folder: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ResolveLibrary {
    pub database: Option<ResolveDatabase>,
    pub databases: Vec<ResolveDatabase>,
    pub projects: Vec<LibraryProject>,
}

/// Where a new project comes from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProjectSource {
    Open,
    Library {
        name: String,
        folder: Vec<String>,
        database: Option<ResolveDatabase>,
    },
    /// A Blackmagic Cloud project by name. Linker-Edit never opens it: you open it in Resolve yourself.
    Cloud {
        name: String,
    },
    /// A working folder that already has a linker-edit.json, such as a teammate's cloned history.
    Folder {
        dir: String,
    },
}

/// One folder in a FolderListing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FolderEntry {
    pub name: String,
    /// Holds a linker-edit.json, so it can be added as a project folder.
    pub is_project: bool,
}

/// The folders inside one folder on the server's computer, for picking a path without typing it all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    /// Absolute, ending in a separator, such as C:\Users\you\.
    pub dir: String,
    pub is_project: bool,
    pub entries: Vec<FolderEntry>,
    /// The drives on Windows, or / elsewhere: where to jump from the picker.
    pub roots: Vec<String>,
    pub home: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovalRequest {
    pub id: String,
    pub tool: String,
    #[serde(default)]
    #[ts(type = "unknown")]
    pub input: Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ClipChange {
    Added,
    Removed,
    Trimmed,
    Moved,
    Changed,
}

/// A clip on a history strip, or a run of untouched clips folded into a count.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum StripTile {
    Clip {
        name: String,
        /// Media pool item id, for its thumbnail at /media/<id>; null for titles and generators.
        media: Option<String>,
        image: bool,
        change: Option<ClipChange>,
        /// Seconds a clip moved, or its change in length when trimmed.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            serialize_with = "number::option"
        )]
        #[ts(optional)]
        delta: Option<f64>,
    },
    More {
        count: u32,
    },
}

/// Whether a TimelineStrip's timeline is new in the save: "new" | "changed".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum TimelineStripState {
    New,
    Changed,
}

/// A timeline's video clips in a save: all of them for a new timeline, the changed ones in context otherwise.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TimelineStrip {
    /// Resolve's timeline id, for previewing it.
    pub id: String,
    pub name: String,
    #[ts(inline)]
    pub state: TimelineStripState,
    pub tiles: Vec<StripTile>,
}

/// One save in the workspace's edit history, newest first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct HistoryEntry {
    pub commit: String,
    /// ISO 8601.
    pub date: String,
    pub author: String,
    /// The agent that made the change; null for the user's own changes in Resolve.
    pub agent: Option<Provider>,
    pub subject: String,
    pub prompt: Option<String>,
    pub actions: Vec<String>,
    pub changes: Vec<String>,
    /// The Linker-Edit thread the agent's turn ran in; null for other saves.
    pub thread: Option<String>,
    pub timelines: Vec<TimelineStrip>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum MediaKind {
    Video,
    Image,
    Audio,
    Unsupported,
    Missing,
}

/// One of a clip's look values: a number, or a switch such as Flip.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, TS)]
#[serde(untagged)]
pub enum LookValue {
    Number(f64),
    Bool(bool),
}

impl Serialize for LookValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            LookValue::Number(value) => number::serialize(value, serializer),
            LookValue::Bool(value) => serializer.serialize_bool(*value),
        }
    }
}

/// A clip as the quick preview plays it. Times are seconds from the timeline's start.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlayableClip {
    pub name: String,
    pub track: u32,
    #[serde(serialize_with = "number::serialize")]
    pub start: f64,
    #[serde(serialize_with = "number::serialize")]
    pub end: f64,
    /// Seconds into the source where the clip starts.
    #[serde(serialize_with = "number::serialize")]
    pub offset: f64,
    /// Playback speed, 1 for normal.
    #[serde(serialize_with = "number::serialize")]
    pub rate: f64,
    /// Media pool item id, played from /media/<id>; null for titles and generators.
    pub media: Option<String>,
    pub kind: MediaKind,
    /// Resolve's transform and opacity values that differ from the defaults (ZoomX, Pan, Opacity…).
    #[ts(type = "Record<string, number | boolean>")]
    pub look: BTreeMap<String, LookValue>,
}

/// A marker on a PlayableTimeline.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TimelineMarker {
    #[serde(serialize_with = "number::serialize")]
    pub time: f64,
    pub color: String,
    pub name: String,
}

/// A timeline by its Resolve id and name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct TimelineRef {
    pub id: String,
    pub name: String,
}

/// A timeline the browser can play straight from its media, without rendering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PlayableTimeline {
    pub id: String,
    pub name: String,
    /// The save it comes from; null for the timeline as it is in Resolve now.
    pub commit: Option<String>,
    #[serde(serialize_with = "number::serialize")]
    pub fps: f64,
    pub width: u32,
    pub height: u32,
    #[serde(serialize_with = "number::serialize")]
    pub duration: f64,
    pub video: Vec<PlayableClip>,
    pub audio: Vec<PlayableClip>,
    #[ts(inline)]
    pub markers: Vec<TimelineMarker>,
    /// For an AI timeline as Resolve has it now: the main timeline it's a new version of, which it can replace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, inline)]
    pub replaces: Option<TimelineRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct PreviewTimeline {
    pub id: String,
    pub name: String,
}

/// A small render of one timeline, made by Resolve and played in the preview panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum PreviewState {
    Idle,
    /// The timeline is null until Resolve picks up the job.
    Rendering {
        timeline: Option<PreviewTimeline>,
        #[serde(serialize_with = "number::serialize")]
        percent: f64,
    },
    Ready {
        timeline: PreviewTimeline,
        url: String,
        rendered_at: String,
        /// Resolve was changed through Linker-Edit after this render.
        outdated: bool,
    },
    Error {
        message: String,
    },
}

/// How full the agent's context was after its latest request. Claude only: Codex reports no window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub used_tokens: u32,
    /// Null until the turn's result reports the model's window.
    pub max_tokens: Option<u32>,
    /// When Claude's prompt cache expires; a turn after that rereads the whole chat uncached. ISO 8601.
    pub cache_expires_at: Option<String>,
}

/// Below this, rereading a chat after its cache expired is cheap, so compacting isn't suggested. As in T3 Code.
pub const COMPACT_SUGGESTION_MIN_TOKENS: u32 = 200_000;

/// Speech-to-text work on this machine. Percent is null while unpacking; Whisper itself reports no progress.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum TranscriptionStatus {
    /// ready: whether the speech engine is downloaded; left out while unknown.
    Idle {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        ready: Option<bool>,
    },
    Preparing {
        step: String,
        #[serde(serialize_with = "number::option")]
        percent: Option<f64>,
    },
    Transcribing {
        file: String,
        done: u32,
        total: u32,
    },
}

/// Opening this app from the user's other devices through Tailscale Serve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum RemoteAccess {
    Checking,
    /// Tailscale isn't installed, running or logged in here; hint says what to do.
    Unavailable {
        hint: String,
    },
    /// problem is why turning it on failed or can't work yet, which may include a link.
    Off {
        url: String,
        problem: Option<String>,
    },
    Switching {
        url: String,
    },
    /// Only devices logged in to Tailscale as owner get in.
    On {
        url: String,
        owner: String,
    },
}

/// The Linker account (linker.cool) signed in on this computer. Optional: Linker-Edit works without one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LinkerAccount {
    /// problem is why the last sign-in failed or ended, worded for the user.
    SignedOut { problem: Option<String> },
    /// Waiting for the browser to come back from Linker.
    SigningIn,
    SignedIn {
        name: String,
        email: Option<String>,
        picture: Option<String>,
        /// The Linker plan's name (Free, Pro, Max…); null when Linker didn't say.
        plan: Option<String>,
        /// Linker's dashboard.
        app_url: String,
    },
}

/// How a Computer is reached: "tailscale" | "linker".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ComputerVia {
    Tailscale,
    Linker,
}

/// Another of the user's computers that runs Linker-Edit with remote access on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Computer {
    pub name: String,
    pub url: String,
    pub version: String,
    /// How it's reached: Tailscale (the default for older servers) or Linker Connect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional, inline)]
    pub via: Option<ComputerVia>,
    /// False for a Linker Connect computer that isn't reachable now.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub online: Option<bool>,
}

/// One of the edit computers linked to your Linker account (at most LinkerConnect.limit).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LinkedComputer {
    pub id: String,
    pub name: String,
    /// https://edit-….linker.cool
    pub url: String,
    /// Whether its tunnel is connected now; null when Linker couldn't tell.
    pub online: Option<bool>,
    pub linked_at: String,
    pub last_seen_at: Option<String>,
}

/// How a Linker Connect device stays signed in: Linker-Edit's window with a cookie, or a native app (the iPad app)
/// with a Bearer token.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ConnectDeviceKind {
    Cookie,
    Native,
}

/// A device that opened this computer through Linker Connect, and can be signed out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ConnectDevice {
    pub id: String,
    pub name: String,
    pub kind: ConnectDeviceKind,
    pub created_at: String,
    pub last_seen_at: String,
}

/// signed_out: sign in with Linker first. off. starting: linking, downloading cloudflared or connecting.
/// on: linked, with the tunnel running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LinkerConnectState {
    SignedOut,
    Off,
    Starting,
    On,
}

/// Linker Connect, like T3 Connect: this computer gets its own address at Linker, through a Cloudflare Tunnel,
/// and your other computers signed in to the same Linker account open it from anywhere.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LinkerConnect {
    /// signed_out: sign in with Linker first. off. starting: linking, downloading cloudflared or connecting.
    /// on: linked, with the tunnel running.
    #[ts(inline)]
    pub state: LinkerConnectState,
    /// This computer's address once linked.
    pub url: Option<String>,
    /// While on: whether the tunnel is connected to Cloudflare right now.
    pub connected: bool,
    /// What starting is doing, such as "Downloading cloudflared · 40%".
    pub step: Option<String>,
    /// Why it isn't on, or what went wrong last, worded for the user.
    pub problem: Option<String>,
    /// This computer's id at Linker, to find it among computers.
    pub computer_id: String,
    /// Your linked computers, this one included when linked; null until looked up.
    pub computers: Option<Vec<LinkedComputer>>,
    /// How many computers one Linker account can link.
    pub limit: u32,
    /// Linking stopped because the limit is reached: remove one of computers first.
    pub at_limit: bool,
    /// Devices signed in to this computer through Linker Connect.
    pub devices: Vec<ConnectDevice>,
}

/// GET /api/computers: this computer's name, and the user's other computers found over Tailscale.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Computers {
    pub name: String,
    /// This computer's Linker-Edit; missing from older ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub version: Option<String>,
    pub others: Vec<Computer>,
    /// Added by the desktop app: this computer's own Linker-Edit address.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub home: Option<String>,
}

/// What the first desktop apps told their pages about an update; the preload still offers it to pages of older
/// Linker-Edits on the user's other computers. "ready" installs on restart; "download" means download it by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "state",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DesktopUpdate {
    Ready { version: String },
    Download { version: String, url: String },
}

/// Where a DesktopUpdateState is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum DesktopUpdateStatus {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading,
    Downloaded,
    Error,
}

/// Where the installed desktop app is with its own updates. desktop/update-state.ts moves it from one status to the
/// next; the app's server passes it on to every page, so another of the user's computers shows its own.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct DesktopUpdateState {
    #[ts(inline)]
    pub status: DesktopUpdateStatus,
    /// The version running now.
    pub current_version: String,
    /// The newest release, once a check found one.
    pub available_version: Option<String>,
    /// The release downloaded and ready to install on restart.
    pub downloaded_version: Option<String>,
    /// While downloading.
    #[serde(serialize_with = "number::option")]
    pub percent: Option<f64>,
    /// What's new in the available release, a few short lines.
    pub notes: Vec<String>,
    /// When the last check finished, as an ISO date.
    pub checked_at: Option<String>,
    /// What went wrong, for the "error" status, or an install that failed with an update still downloaded.
    pub error: Option<String>,
    /// The release page, when the app can't install it by itself: unsigned Mac apps can't replace themselves, so
    /// the update is downloaded by hand.
    pub download_url: Option<String>,
    /// An install of this version was started, but the app came back on the old version.
    pub not_applied: Option<String>,
}

/// AppMeta.appUpdate: the desktop app's update state, plus what this Linker-Edit is doing about it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AppUpdate {
    #[serde(flatten)]
    pub state: DesktopUpdateState,
    /// The user asked to restart to update while Linker-Edit was busy with Resolve; it restarts once it's done.
    pub waiting: bool,
}

/// ComputerSetup.folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct SetupFolders {
    pub projects: String,
    pub previews: String,
    pub transcripts: String,
    pub tools: String,
}

/// Where this computer's Linker-Edit keeps things, for the settings page. Set in linker-edit.config.json.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ComputerSetup {
    /// The config file the paths come from; it may not exist yet.
    pub config_file: String,
    #[ts(inline)]
    pub folders: SetupFolders,
    /// Resolve's own MCP server program, found or configured; null when this computer has none.
    pub resolve_mcp: Option<String>,
}

/// AppMeta.blocked: why a message can't go to the agent, and whether opening the project in Resolve fixes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Blocked {
    pub message: String,
    pub can_open: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AppMeta {
    pub projects: Vec<ProjectSummary>,
    /// The thread this client is looking at; null when there are no projects.
    pub thread_id: Option<String>,
    pub project_id: Option<String>,
    pub provider: Provider,
    /// The agent new threads start with.
    pub default_provider: Provider,
    pub approval_mode: ApprovalMode,
    /// This thread's turn is running.
    pub busy: bool,
    pub approvals: Vec<ApprovalRequest>,
    pub resolve: ResolveStatus,
    /// False when the user said this computer has no Resolve: it only drives an edit rig, so Resolve's setup
    /// and the speech engine aren't offered here. Becomes true again once Resolve is found on it.
    pub resolve_here: bool,
    pub version: String,
    /// A newer Linker-Edit on npm, or null. For `npx linker-edit`; the desktop app has appUpdate instead.
    pub update: Option<String>,
    /// The updates of the desktop app this Linker-Edit runs in; null when it isn't one, as with `npx`.
    pub app_update: Option<AppUpdate>,
    /// The thread's project folder.
    pub workspace_dir: Option<String>,
    pub preview: PreviewState,
    pub context: Option<ContextUsage>,
    /// Why a message in this thread can't go to the agent right now; null when it can.
    #[ts(inline)]
    pub blocked: Option<Blocked>,
    /// Linker-Edit is opening this project in Resolve.
    pub opening: Option<String>,
    /// Linker-Edit is making an AI timeline of this project its main one.
    pub applying: Option<String>,
    pub transcription: TranscriptionStatus,
    pub remote: RemoteAccess,
    pub connect: LinkerConnect,
    /// The name of the computer this Linker-Edit runs on.
    pub computer: String,
    /// Null in tests and builds that don't say.
    pub setup: Option<ComputerSetup>,
    pub linker: LinkerAccount,
    /// Every workspace and series, workspaces first, each in the order it was made.
    pub collections: Vec<CollectionSummary>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    #[serde(flatten)]
    pub meta: AppMeta,
    pub items: Vec<ChatItem>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ClientMessage {
    /// compactFirst summarizes the chat before sending: for coming back after the prompt cache expired.
    Send {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        compact_first: Option<bool>,
    },
    /// Summarizes the chat so far to free context. Claude only.
    Compact,
    Stop,
    SetProvider {
        provider: Provider,
    },
    /// The agent new threads start with, without changing the open thread's.
    SetDefaultProvider {
        provider: Provider,
    },
    SetApprovalMode {
        mode: ApprovalMode,
    },
    Approve {
        id: String,
        allow: bool,
    },
    RefreshResolve,
    /// Whether this computer has Resolve (or will); see AppMeta.resolveHere.
    SetResolveHere {
        resolve_here: bool,
    },
    LoadHistory,
    /// PROJECT.md of the project in view: what the video is, kept by the agent and edited here by the user.
    LoadProjectFile,
    SaveProjectFile {
        project_id: String,
        text: String,
    },
    /// Without a timeline id, renders Resolve's current timeline.
    RenderPreview {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        timeline_id: Option<String>,
    },
    /// The user is happy with an AI timeline: it takes the main timeline's place, which is kept renamed.
    ApplyTimeline {
        project_id: String,
        timeline_id: String,
    },
    StopPreview,
    SelectThread {
        thread_id: String,
    },
    NewThread {
        project_id: String,
    },
    RenameThread {
        thread_id: String,
        title: String,
    },
    ArchiveThread {
        thread_id: String,
        archived: bool,
    },
    DeleteThread {
        thread_id: String,
    },
    AddProject {
        source: ProjectSource,
    },
    UpdateProject {
        project_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        name: Option<String>,
        #[serde(flatten)]
        changes: ProjectSidebarChanges,
    },
    /// Links the project to the one open in Resolve now.
    RelinkProject {
        project_id: String,
    },
    /// Takes the project out of Linker-Edit. Its folder and its Resolve project stay.
    RemoveProject {
        project_id: String,
    },
    /// Saves the project open in Resolve, then opens this one.
    OpenInResolve {
        project_id: String,
    },
    LoadLibrary,
    /// Lists the folders in path on the server's computer; an empty path is the home folder.
    BrowseFolders {
        request_id: u32,
        path: String,
    },
    RefreshRemoteAccess,
    SetRemoteAccess {
        enabled: bool,
    },
    /// Looks up the computers linked to your Linker account again.
    RefreshConnect,
    SetConnect {
        enabled: bool,
    },
    /// Unlinks one of your computers from Linker Connect, this one or another, which frees its place.
    ConnectUnlink {
        computer_id: String,
    },
    /// Signs a device out of this computer.
    ConnectSignOutDevice {
        device_id: String,
    },
    /// Stops waiting for a Linker sign-in started in the browser (GET /api/linker/sign-in starts one).
    LinkerCancelSignIn,
    LinkerSignOut,
    /// Downloads the speech engine now, or stops that download.
    TranscriptionSetup,
    TranscriptionCancel,
    /// Looks for a new release of the desktop app now.
    CheckAppUpdate,
    /// Restarts the desktop app into the downloaded release, once nothing is busy with Resolve.
    InstallAppUpdate,
    /// Doesn't restart after all when Linker-Edit is done.
    CancelAppUpdate,
    #[serde(untagged)]
    Library(LibraryClientMessage),
    #[serde(untagged)]
    Collection(CollectionMessage),
    #[serde(untagged)]
    Sidebar(SidebarClientMessage),
}

/// Between the desktop app (desktop/main.ts) and the server it starts, over the child process's IPC channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum DesktopToServer {
    UpdateState {
        state: Box<DesktopUpdateState>,
    },
    /// A page of an older Linker-Edit asked to restart to update; the server waits until Resolve is free.
    InstallRequested,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ServerToDesktop {
    Ready {
        url: String,
    },
    CheckUpdate,
    /// Nothing is busy with Resolve: install now.
    InstallUpdate,
}

/// A result or why there is none, as `T | { error: string }` on /ws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
pub enum OrError<T> {
    Error { error: String },
    Ok(T),
}

impl<T> From<Result<T, String>> for OrError<T> {
    fn from(result: Result<T, String>) -> Self {
        match result {
            Ok(value) => OrError::Ok(value),
            Err(error) => OrError::Error { error },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ServerMessage {
    Snapshot {
        state: Box<AppState>,
    },
    Meta {
        meta: Box<AppMeta>,
    },
    Item {
        thread_id: String,
        item: ChatItem,
    },
    History {
        project_id: String,
        entries: Vec<HistoryEntry>,
    },
    /// A project's PROJECT.md: on request, after a save, and after each turn in the project. file is its path.
    ProjectFile {
        project_id: String,
        file: String,
        text: String,
    },
    Library {
        #[ts(inline)]
        library: OrError<ResolveLibrary>,
    },
    Folders {
        request_id: u32,
        #[ts(inline)]
        listing: OrError<FolderListing>,
    },
    /// The message wasn't sent; it goes back in the composer.
    Refused {
        thread_id: String,
        text: String,
    },
    Toast {
        text: String,
        #[ts(inline)]
        tone: Tone,
    },
    #[serde(untagged)]
    LibraryView(LibraryServerMessage),
}

// --- Library ---
// Folders of files the user uses again and again (music, sound effects, logos, LUTs…). Linker-Edit only points
// at them: it never copies, moves or deletes a file in them.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryKind {
    Music,
    Sfx,
    Graphics,
    Luts,
    Footage,
    Other,
}

pub const LIBRARY_KINDS: [LibraryKind; 6] = [
    LibraryKind::Music,
    LibraryKind::Sfx,
    LibraryKind::Graphics,
    LibraryKind::Luts,
    LibraryKind::Footage,
    LibraryKind::Other,
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolder {
    pub id: String,
    pub kind: LibraryKind,
    /// "Epidemic downloads", "Brand logos".
    pub label: String,
    /// This machine's path to it; kept in this machine's files, never committed.
    pub path: String,
    /// "licensed for YouTube only", "use for intros". Agents read it.
    pub note: String,
}

/// A LibraryFolder without its id, as a new one is sent: `Omit<LibraryFolder, "id">`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolderFields {
    pub kind: LibraryKind,
    pub label: String,
    pub path: String,
    pub note: String,
}

/// What changes in a LibraryFolder: `Partial<Omit<LibraryFolder, "id">>`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolderChanges {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<LibraryKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub note: Option<String>,
}

/// Which library. Most specific last in a chain: universal, then (later) workspace and series, then the project.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LibraryScope {
    Universal,
    Project { project_id: String },
}

/// `LibraryScope["kind"]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum LibraryScopeKind {
    Universal,
    Project,
}

/// A project and timeline a library file was brought into by an agent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryUse {
    pub project_id: String,
    pub project: String,
    pub timeline: Option<String>,
    /// ISO 8601.
    pub at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFile {
    pub folder_id: String,
    #[ts(type = "LibraryScope[\"kind\"]")]
    pub scope: LibraryScopeKind,
    pub kind: LibraryKind,
    /// The path inside its folder, with / between folders. Plays from /library/<folderId>/<rel>.
    pub rel: String,
    pub name: String,
    /// Absolute, on the computer Linker-Edit runs on.
    pub path: String,
    #[ts(type = "number")]
    pub size: u64,
    /// ISO 8601.
    pub modified: String,
    /// Seconds; null for stills and files ffmpeg can't read.
    #[serde(serialize_with = "number::option")]
    pub duration: Option<f64>,
    pub channels: Option<u32>,
    /// Integrated loudness in LUFS; null when not measured (stills, footage, no audio).
    #[serde(serialize_with = "number::option")]
    pub loudness: Option<f64>,
    pub tags: Vec<String>,
    pub star: bool,
    pub note: String,
    pub used_in: Vec<LibraryUse>,
}

/// LibraryFolderStatus.scanning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    pub done: u32,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryFolderStatus {
    #[serde(flatten)]
    pub folder: LibraryFolder,
    /// Files found by the last scan.
    pub files: u32,
    /// While a scan runs.
    #[ts(inline)]
    pub scanning: Option<ScanProgress>,
    /// ISO 8601; null before the first scan.
    pub scanned_at: Option<String>,
    /// Why the last scan failed, such as a folder that's gone.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryQuery {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub kind: Option<LibraryKind>,
    /// Words to find in the file's name, tags and note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub query: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub tags: Option<Vec<String>>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "number::option"
    )]
    #[ts(optional)]
    pub min_seconds: Option<f64>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        serialize_with = "number::option"
    )]
    #[ts(optional)]
    pub max_seconds: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub starred: Option<bool>,
}

/// LibraryView.scopes: a library the page can switch to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryScopeOption {
    pub scope: LibraryScope,
    pub label: String,
}

/// One library as the library page shows it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct LibraryView {
    pub scope: LibraryScope,
    /// The libraries this page can switch between, least specific first.
    #[ts(inline)]
    pub scopes: Vec<LibraryScopeOption>,
    pub folders: Vec<LibraryFolderStatus>,
    /// The first matches; total says how many there are.
    pub files: Vec<LibraryFile>,
    pub total: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LibraryClientMessage {
    /// Shows a library, and keeps sending it while it changes, until library_close.
    LibraryOpen {
        scope: LibraryScope,
        query: LibraryQuery,
    },
    LibraryClose,
    LibraryAddFolder {
        scope: LibraryScope,
        #[ts(type = "Omit<LibraryFolder, \"id\">")]
        folder: LibraryFolderFields,
    },
    LibraryUpdateFolder {
        scope: LibraryScope,
        folder_id: String,
        #[ts(type = "Partial<Omit<LibraryFolder, \"id\">>")]
        changes: LibraryFolderChanges,
    },
    /// Forgets the folder. The folder and its files stay where they are.
    LibraryRemoveFolder {
        scope: LibraryScope,
        folder_id: String,
    },
    LibraryRescan {
        folder_id: String,
    },
    LibraryUpdateFile {
        folder_id: String,
        rel: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        star: Option<bool>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        tags: Option<Vec<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        note: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum LibraryServerMessage {
    LibraryView { view: LibraryView },
}

// --- Workspaces & series ---

/// A workspace is a client, brand or channel; a series holds episodes. The UI words are in src/lib/labels.ts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum CollectionKind {
    Workspace,
    Series,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum IconColor {
    Gray,
    Red,
    Orange,
    Amber,
    Green,
    Teal,
    Blue,
    Violet,
    Pink,
}

/// Like T3 Code's project icons: a lucide icon, an emoji or a few letters, on a color.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CollectionIcon {
    /// name is the icon's name in lucide, such as "mic" or "clapperboard".
    Lucide {
        name: String,
        color: IconColor,
    },
    Emoji {
        emoji: String,
        color: IconColor,
    },
    Monogram {
        text: String,
        color: IconColor,
    },
}

/// CollectionSummary.examples: notes on one example video.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollectionExample {
    pub name: String,
    pub file: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct CollectionSummary {
    pub id: String,
    pub kind: CollectionKind,
    pub name: String,
    /// The workspace a series is in; always null for a workspace. One level deep.
    pub parent_id: Option<String>,
    pub icon: CollectionIcon,
    /// The style guide for the agent, in plain words; its RULES.md.
    pub rules: String,
    /// Where the rules are kept, to edit them in another editor too.
    pub rules_file: String,
    /// Projects directly in it, not those in its series.
    pub project_ids: Vec<String>,
    /// Its projects' agent makes the first cut without asking, and says afterwards what it decided.
    pub autonomous: bool,
    /// What feedback taught the agent, oldest first. Each was accepted by the user.
    pub lessons: Vec<Lesson>,
    /// The agent's notes on example videos it studied, kept as files next to the rules.
    #[ts(inline)]
    pub examples: Vec<CollectionExample>,
    /// The project that holds its setup threads; null until "Set up with AI" was used.
    pub setup_project_id: Option<String>,
    /// Approved edits of its projects, newest first, to see whether the brief works.
    pub edits: Vec<ApprovedEdit>,
}

/// A rule the agent learned from the user's feedback, kept with the words it came from so a wrong one is easy to spot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct Lesson {
    pub id: String,
    pub rule: String,
    /// What the user said, quoted.
    pub feedback: String,
    /// ISO 8601.
    pub added_at: String,
    /// The project it was learned in.
    pub project: Option<String>,
}

/// An AI timeline the user made the main one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ApprovedEdit {
    /// ISO 8601.
    pub at: String,
    pub project: String,
    /// Messages with feedback the user sent before approving; 0 when the first cut was right.
    pub rounds: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum CollectionMessage {
    /// A series may go in a workspace; a workspace goes in nothing.
    CreateCollection {
        kind: CollectionKind,
        name: String,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        #[ts(optional)]
        parent_id: Option<Option<String>>,
        /// An icon that isn't one is left out, so the collection gets one from its name.
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "lenient"
        )]
        #[ts(optional)]
        icon: Option<CollectionIcon>,
    },
    UpdateCollection {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        name: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        icon: Option<CollectionIcon>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        rules: Option<String>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "present"
        )]
        #[ts(optional)]
        parent_id: Option<Option<String>>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[ts(optional)]
        autonomous: Option<bool>,
    },
    /// Forgets a lesson or the notes on an example.
    RemoveLesson {
        id: String,
        lesson_id: String,
    },
    RemoveExample {
        id: String,
        name: String,
    },
    /// Opens its setup thread, where the agent studies example videos and writes the brief; made on first use.
    SetupCollection {
        id: String,
    },
    /// Refused while projects or series are in it.
    DeleteCollection {
        id: String,
    },
    /// Puts a project in a workspace or series, or in none.
    MoveProject {
        project_id: String,
        collection_id: Option<String>,
    },
}

// --- Sidebar (T3 Code) ---
// Pinning, settling, snoozing and unread threads, and project icons, as T3 Code's sidebar has them (MIT, see NOTICE).

/// T3 Code's project icon colors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
pub enum ProjectIconColor {
    Gray,
    Red,
    Orange,
    Amber,
    Yellow,
    Lime,
    Green,
    Emerald,
    Teal,
    Cyan,
    Sky,
    Blue,
    Indigo,
    Violet,
    Purple,
    Fuchsia,
    Pink,
    Rose,
}

/// T3 Code's project icon colors.
pub const PROJECT_ICON_COLORS: [ProjectIconColor; 18] = [
    ProjectIconColor::Gray,
    ProjectIconColor::Red,
    ProjectIconColor::Orange,
    ProjectIconColor::Amber,
    ProjectIconColor::Yellow,
    ProjectIconColor::Lime,
    ProjectIconColor::Green,
    ProjectIconColor::Emerald,
    ProjectIconColor::Teal,
    ProjectIconColor::Cyan,
    ProjectIconColor::Sky,
    ProjectIconColor::Blue,
    ProjectIconColor::Indigo,
    ProjectIconColor::Violet,
    ProjectIconColor::Purple,
    ProjectIconColor::Fuchsia,
    ProjectIconColor::Pink,
    ProjectIconColor::Rose,
];

/// A project's icon in the sidebar: a Lucide icon, an emoji, or one or two letters, as in T3 Code.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ProjectIcon {
    Lucide {
        name: String,
        color: ProjectIconColor,
    },
    Emoji {
        emoji: String,
    },
    Monogram {
        text: String,
        color: ProjectIconColor,
    },
}

/// Where a thread sits in the sidebar. All ISO 8601, null when it never happened; missing from older files.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ThreadSidebarState {
    pub created_at: String,
    pub pinned_at: Option<String>,
    /// Settled threads leave the project's list for the Settled shelf, and stay searchable.
    pub settled_at: Option<String>,
    /// When it was last taken out of Settled.
    pub unsettled_at: Option<String>,
    /// Hidden in the Snoozed shelf until then.
    pub snoozed_until: Option<String>,
    pub snoozed_at: Option<String>,
    /// When someone last looked at it; a turn that ended after that is unread.
    pub last_read_at: Option<String>,
    /// When the user last sent a message in it: threads sort by this.
    pub last_sent_at: Option<String>,
    /// When its last turn ended.
    pub turn_ended_at: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSidebarState {
    /// Null shows a monogram made from the name.
    pub icon: Option<ProjectIcon>,
    /// Pinned projects come first and stay listed with no active threads.
    pub pinned: bool,
    /// Archived projects go to the end, and only while they have active threads.
    pub archived: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
pub struct ProjectSidebarChanges {
    /// null takes the icon away, and so does an icon parseProjectIcon refuses: the project falls back to its monogram.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::project_icon::deserialize_change"
    )]
    #[ts(optional)]
    pub icon: Option<Option<ProjectIcon>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub pinned: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub archived: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum SidebarClientMessage {
    PinThread {
        thread_id: String,
        pinned: bool,
    },
    /// Settling also unpins and wakes it, like T3 Code.
    SettleThread {
        thread_id: String,
        settled: bool,
    },
    /// A null until wakes it now.
    SnoozeThread {
        thread_id: String,
        until: Option<String>,
    },
    /// read: false marks the last turn unread again.
    MarkRead {
        thread_id: String,
        read: bool,
    },
}

/// For `field?: T | null`: a field that's there, even as null, is Some, so null and left out differ.
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// A value that doesn't fit is left out instead of failing the whole message.
fn lenient<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::de::DeserializeOwned,
{
    let value = Value::deserialize(deserializer)?;
    Ok(serde_json::from_value(value).ok())
}
