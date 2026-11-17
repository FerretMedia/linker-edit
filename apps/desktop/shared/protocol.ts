// Messages exchanged between the local server and the browser UI over /ws.
//
// Generated from crates/protocol: don't edit it here. Change the Rust types, then run
// `UPDATE_PROTOCOL_TS=1 cargo test -p linker-edit-protocol` to write this file again.

export type Provider = "claude" | "codex";

/**
 * Like T3 Code's runtime modes. "auto" (the default) is full access: everything allowed runs without asking.
 * "ask" is supervised: Resolve changes, scripts and file edits wait for the user. What the policy refuses,
 * it refuses in both.
 */
export type ApprovalMode = "ask" | "auto";

export type ToolStatus = "running" | "done" | "error";

export type ChatItem =
  | { kind: "user"; id: string; text: string }
  | { kind: "assistant"; id: string; text: string }
  | { kind: "tool"; id: string; name: string; input: unknown; status: ToolStatus; output?: string }
  | { kind: "notice"; id: string; text: string; tone: "info" | "error" };

export type ResolveStatus =
  | { state: "checking" }
  | {
    state: "connected";
    product: string;
    version: string;
    project: string | null;
    /** Resolve's unique id for the open project; null when none is open or Resolve doesn't say. */
    projectId: string | null;
    /** The project library the open project lives in. */
    database: ResolveDatabase | null;
    timeline: string | null;
  }
  | { state: "not_running" }
  | { state: "unreachable"; hint: string }
  | { state: "blocked"; hint: string }
  | { state: "setup_needed"; hint: string };

/** A project library as Resolve's GetCurrentDatabase() reports it. */
export type ResolveDatabase = {
  /** "Disk" or "PostgreSQL" in Resolve's documentation; anything else is treated as a cloud library. */
  type: string;
  name: string;
  /** PostgreSQL server address. */
  host?: string;
};

/**
 * Which Resolve project a Linker-Edit project belongs to. Only what is the same on every machine,
 * so it can be committed and shared; see LocalLink for the rest.
 */
export type ResolveLink = {
  kind: "local" | "network" | "cloud";
  name: string;
  /** Resolve's unique project id once Linker-Edit has seen the project open; null before. */
  projectId: string | null;
};

/** This machine's side of a link: where to find the project, never committed. */
export type LocalLink = {
  database?: ResolveDatabase;
  /** Project Manager folders down to the project. */
  folder?: Array<string>;
};

export type ThreadSummary = {
  id: string;
  title: string;
  /** ISO 8601. */
  updatedAt: string;
  archived: boolean;
  busy: boolean;
  /** An approval is waiting for the user. */
  waiting: boolean;
  createdAt: string;
  pinnedAt: string | null;
  /** Settled threads leave the project's list for the Settled shelf, and stay searchable. */
  settledAt: string | null;
  /** When it was last taken out of Settled. */
  unsettledAt: string | null;
  /** Hidden in the Snoozed shelf until then. */
  snoozedUntil: string | null;
  snoozedAt: string | null;
  /** When someone last looked at it; a turn that ended after that is unread. */
  lastReadAt: string | null;
  /** When the user last sent a message in it: threads sort by this. */
  lastSentAt: string | null;
  /** When its last turn ended. */
  turnEndedAt: string | null;
};

export type ProjectSummary = {
  id: string;
  name: string;
  /** The project's working folder: its edit history, notes and exports. */
  dir: string;
  /** Null until the project's first turn with Resolve open, or until it is linked by hand. */
  link: ResolveLink | null;
  local: LocalLink;
  /** Its Resolve project is the one open in Resolve now. */
  open: boolean;
  /** Newest first, archived ones included. */
  threads: Array<ThreadSummary>;
  /**
   * A workspace's or series' setup threads: the agent studies example videos and writes the brief here. It has no
   * Resolve project and never gets one.
   */
  setup: boolean;
  /** Null shows a monogram made from the name. */
  icon: ProjectIcon | null;
  /** Pinned projects come first and stay listed with no active threads. */
  pinned: boolean;
  /** Archived projects go to the end, and only while they have active threads. */
  archived: boolean;
};

/** A project in the Resolve database that is open now, for adding it to Linker-Edit. */
export type LibraryProject = { name: string; folder: Array<string> };

export type ResolveLibrary = {
  database: ResolveDatabase | null;
  databases: Array<ResolveDatabase>;
  projects: Array<LibraryProject>;
};

/** Where a new project comes from. */
export type ProjectSource =
  | { kind: "open" }
  | { kind: "library"; name: string; folder: Array<string>; database: ResolveDatabase | null }
  | { kind: "cloud"; name: string }
  | { kind: "folder"; dir: string };

/** One folder in a FolderListing. */
export type FolderEntry = {
  name: string;
  /** Holds a linker-edit.json, so it can be added as a project folder. */
  isProject: boolean;
};

/** The folders inside one folder on the server's computer, for picking a path without typing it all. */
export type FolderListing = {
  /** Absolute, ending in a separator, such as C:\Users\you\. */
  dir: string;
  isProject: boolean;
  entries: Array<FolderEntry>;
  /** The drives on Windows, or / elsewhere: where to jump from the picker. */
  roots: Array<string>;
  home: string;
};

export type ApprovalRequest = { id: string; tool: string; input: unknown };

export type ClipChange = "added" | "removed" | "trimmed" | "moved" | "changed";

/** A clip on a history strip, or a run of untouched clips folded into a count. */
export type StripTile =
  | {
    kind: "clip";
    name: string;
    /** Media pool item id, for its thumbnail at /media/<id>; null for titles and generators. */
    media: string | null;
    image: boolean;
    change: ClipChange | null;
    /** Seconds a clip moved, or its change in length when trimmed. */
    delta?: number;
  }
  | { kind: "more"; count: number };

/** A timeline's video clips in a save: all of them for a new timeline, the changed ones in context otherwise. */
export type TimelineStrip = {
  /** Resolve's timeline id, for previewing it. */
  id: string;
  name: string;
  state: "new" | "changed";
  tiles: Array<StripTile>;
};

/** One save in the workspace's edit history, newest first. */
export type HistoryEntry = {
  commit: string;
  /** ISO 8601. */
  date: string;
  author: string;
  /** The agent that made the change; null for the user's own changes in Resolve. */
  agent: Provider | null;
  subject: string;
  prompt: string | null;
  actions: Array<string>;
  changes: Array<string>;
  /** The Linker-Edit thread the agent's turn ran in; null for other saves. */
  thread: string | null;
  timelines: Array<TimelineStrip>;
};

export type MediaKind = "video" | "image" | "audio" | "unsupported" | "missing";

/** A clip as the quick preview plays it. Times are seconds from the timeline's start. */
export type PlayableClip = {
  name: string;
  track: number;
  start: number;
  end: number;
  /** Seconds into the source where the clip starts. */
  offset: number;
  /** Playback speed, 1 for normal. */
  rate: number;
  /** Media pool item id, played from /media/<id>; null for titles and generators. */
  media: string | null;
  kind: MediaKind;
  /** Resolve's transform and opacity values that differ from the defaults (ZoomX, Pan, Opacity…). */
  look: Record<string, number | boolean>;
};

/** A timeline the browser can play straight from its media, without rendering. */
export type PlayableTimeline = {
  id: string;
  name: string;
  /** The save it comes from; null for the timeline as it is in Resolve now. */
  commit: string | null;
  fps: number;
  width: number;
  height: number;
  duration: number;
  video: Array<PlayableClip>;
  audio: Array<PlayableClip>;
  markers: Array<{ time: number; color: string; name: string }>;
  /** For an AI timeline as Resolve has it now: the main timeline it's a new version of, which it can replace. */
  replaces?: { id: string; name: string };
};

export type PreviewTimeline = { id: string; name: string };

/** A small render of one timeline, made by Resolve and played in the preview panel. */
export type PreviewState =
  | { state: "idle" }
  | { state: "rendering"; timeline: PreviewTimeline | null; percent: number }
  | {
    state: "ready";
    timeline: PreviewTimeline;
    url: string;
    renderedAt: string;
    /** Resolve was changed through Linker-Edit after this render. */
    outdated: boolean;
  }
  | { state: "error"; message: string };

/** How full the agent's context was after its latest request. Claude only: Codex reports no window. */
export type ContextUsage = {
  usedTokens: number;
  /** Null until the turn's result reports the model's window. */
  maxTokens: number | null;
  /** When Claude's prompt cache expires; a turn after that rereads the whole chat uncached. ISO 8601. */
  cacheExpiresAt: string | null;
};

/** Below this, rereading a chat after its cache expired is cheap, so compacting isn't suggested. As in T3 Code. */
export const COMPACT_SUGGESTION_MIN_TOKENS = 200_000;

/** Speech-to-text work on this machine. Percent is null while unpacking; Whisper itself reports no progress. */
export type TranscriptionStatus =
  | { state: "idle"; ready?: boolean }
  | { state: "preparing"; step: string; percent: number | null }
  | { state: "transcribing"; file: string; done: number; total: number };

/** Opening this app from the user's other devices through Tailscale Serve. */
export type RemoteAccess =
  | { state: "checking" }
  | { state: "unavailable"; hint: string }
  | { state: "off"; url: string; problem: string | null }
  | { state: "switching"; url: string }
  | { state: "on"; url: string; owner: string };

/** The Linker account (linker.cool) signed in on this computer. Optional: Linker-Edit works without one. */
export type LinkerAccount =
  | { state: "signed_out"; problem: string | null }
  | { state: "signing_in" }
  | {
    state: "signed_in";
    name: string;
    email: string | null;
    picture: string | null;
    /** The Linker plan's name (Free, Pro, Max…); null when Linker didn't say. */
    plan: string | null;
    /** Linker's dashboard. */
    appUrl: string;
  };

/** Another of the user's computers that runs Linker-Edit with remote access on. */
export type Computer = {
  name: string;
  url: string;
  version: string;
  /** How it's reached: Tailscale (the default for older servers) or Linker Connect. */
  via?: "tailscale" | "linker";
  /** False for a Linker Connect computer that isn't reachable now. */
  online?: boolean;
};

/** One of the edit computers linked to your Linker account (at most LinkerConnect.limit). */
export type LinkedComputer = {
  id: string;
  name: string;
  /** https://edit-….linker.cool */
  url: string;
  /** Whether its tunnel is connected now; null when Linker couldn't tell. */
  online: boolean | null;
  linkedAt: string;
  lastSeenAt: string | null;
};

/**
 * How a Linker Connect device stays signed in: Linker-Edit's window with a cookie, or a native app (the iPad app)
 * with a Bearer token.
 */
export type ConnectDeviceKind = "cookie" | "native";

/** A device that opened this computer through Linker Connect, and can be signed out. */
export type ConnectDevice = {
  id: string;
  name: string;
  kind: ConnectDeviceKind;
  createdAt: string;
  lastSeenAt: string;
};

/**
 * Linker Connect, like T3 Connect: this computer gets its own address at Linker, through a Cloudflare Tunnel,
 * and your other computers signed in to the same Linker account open it from anywhere.
 */
export type LinkerConnect = {
  /**
   * signed_out: sign in with Linker first. off. starting: linking, downloading cloudflared or connecting.
   * on: linked, with the tunnel running.
   */
  state: "signed_out" | "off" | "starting" | "on";
  /** This computer's address once linked. */
  url: string | null;
  /** While on: whether the tunnel is connected to Cloudflare right now. */
  connected: boolean;
  /** What starting is doing, such as "Downloading cloudflared · 40%". */
  step: string | null;
  /** Why it isn't on, or what went wrong last, worded for the user. */
  problem: string | null;
  /** This computer's id at Linker, to find it among computers. */
  computerId: string;
  /** Your linked computers, this one included when linked; null until looked up. */
  computers: Array<LinkedComputer> | null;
  /** How many computers one Linker account can link. */
  limit: number;
  /** Linking stopped because the limit is reached: remove one of computers first. */
  atLimit: boolean;
  /** Devices signed in to this computer through Linker Connect. */
  devices: Array<ConnectDevice>;
};

/** GET /api/computers: this computer's name, and the user's other computers found over Tailscale. */
export type Computers = {
  name: string;
  /** This computer's Linker-Edit; missing from older ones. */
  version?: string;
  others: Array<Computer>;
  /** Added by the desktop app: this computer's own Linker-Edit address. */
  home?: string;
};

/**
 * What the first desktop apps told their pages about an update; the preload still offers it to pages of older
 * Linker-Edits on the user's other computers. "ready" installs on restart; "download" means download it by hand.
 */
export type DesktopUpdate =
  | { state: "ready"; version: string }
  | { state: "download"; version: string; url: string };

/**
 * Where the installed desktop app is with its own updates. desktop/update-state.ts moves it from one status to the
 * next; the app's server passes it on to every page, so another of the user's computers shows its own.
 */
export type DesktopUpdateState = {
  status: "idle" | "checking" | "up-to-date" | "available" | "downloading" | "downloaded" | "error";
  /** The version running now. */
  currentVersion: string;
  /** The newest release, once a check found one. */
  availableVersion: string | null;
  /** The release downloaded and ready to install on restart. */
  downloadedVersion: string | null;
  /** While downloading. */
  percent: number | null;
  /** What's new in the available release, a few short lines. */
  notes: Array<string>;
  /** When the last check finished, as an ISO date. */
  checkedAt: string | null;
  /** What went wrong, for the "error" status, or an install that failed with an update still downloaded. */
  error: string | null;
  /**
   * The release page, when the app can't install it by itself: unsigned Mac apps can't replace themselves, so
   * the update is downloaded by hand.
   */
  downloadUrl: string | null;
  /** An install of this version was started, but the app came back on the old version. */
  notApplied: string | null;
};

/** AppMeta.appUpdate: the desktop app's update state, plus what this Linker-Edit is doing about it. */
export type AppUpdate = {
  /** The user asked to restart to update while Linker-Edit was busy with Resolve; it restarts once it's done. */
  waiting: boolean;
  status: "idle" | "checking" | "up-to-date" | "available" | "downloading" | "downloaded" | "error";
  /** The version running now. */
  currentVersion: string;
  /** The newest release, once a check found one. */
  availableVersion: string | null;
  /** The release downloaded and ready to install on restart. */
  downloadedVersion: string | null;
  /** While downloading. */
  percent: number | null;
  /** What's new in the available release, a few short lines. */
  notes: Array<string>;
  /** When the last check finished, as an ISO date. */
  checkedAt: string | null;
  /** What went wrong, for the "error" status, or an install that failed with an update still downloaded. */
  error: string | null;
  /**
   * The release page, when the app can't install it by itself: unsigned Mac apps can't replace themselves, so
   * the update is downloaded by hand.
   */
  downloadUrl: string | null;
  /** An install of this version was started, but the app came back on the old version. */
  notApplied: string | null;
};

/** Where this computer's Linker-Edit keeps things, for the settings page. Set in linker-edit.config.json. */
export type ComputerSetup = {
  /** The config file the paths come from; it may not exist yet. */
  configFile: string;
  folders: { projects: string; previews: string; transcripts: string; tools: string };
  /** Resolve's own MCP server program, found or configured; null when this computer has none. */
  resolveMcp: string | null;
};

export type AppMeta = {
  projects: Array<ProjectSummary>;
  /** The thread this client is looking at; null when there are no projects. */
  threadId: string | null;
  projectId: string | null;
  provider: Provider;
  /** The agent new threads start with. */
  defaultProvider: Provider;
  approvalMode: ApprovalMode;
  /** This thread's turn is running. */
  busy: boolean;
  approvals: Array<ApprovalRequest>;
  resolve: ResolveStatus;
  /**
   * False when the user said this computer has no Resolve: it only drives an edit rig, so Resolve's setup
   * and the speech engine aren't offered here. Becomes true again once Resolve is found on it.
   */
  resolveHere: boolean;
  version: string;
  /** A newer Linker-Edit on npm, or null. For `npx linker-edit`; the desktop app has appUpdate instead. */
  update: string | null;
  /** The updates of the desktop app this Linker-Edit runs in; null when it isn't one, as with `npx`. */
  appUpdate: AppUpdate | null;
  /** The thread's project folder. */
  workspaceDir: string | null;
  preview: PreviewState;
  context: ContextUsage | null;
  /** Why a message in this thread can't go to the agent right now; null when it can. */
  blocked: { message: string; canOpen: boolean } | null;
  /** Linker-Edit is opening this project in Resolve. */
  opening: string | null;
  /** Linker-Edit is making an AI timeline of this project its main one. */
  applying: string | null;
  transcription: TranscriptionStatus;
  remote: RemoteAccess;
  connect: LinkerConnect;
  /** The name of the computer this Linker-Edit runs on. */
  computer: string;
  /** Null in tests and builds that don't say. */
  setup: ComputerSetup | null;
  linker: LinkerAccount;
  /** Every workspace and series, workspaces first, each in the order it was made. */
  collections: Array<CollectionSummary>;
};

export type AppState = {
  items: Array<ChatItem>;
  projects: Array<ProjectSummary>;
  /** The thread this client is looking at; null when there are no projects. */
  threadId: string | null;
  projectId: string | null;
  provider: Provider;
  /** The agent new threads start with. */
  defaultProvider: Provider;
  approvalMode: ApprovalMode;
  /** This thread's turn is running. */
  busy: boolean;
  approvals: Array<ApprovalRequest>;
  resolve: ResolveStatus;
  /**
   * False when the user said this computer has no Resolve: it only drives an edit rig, so Resolve's setup
   * and the speech engine aren't offered here. Becomes true again once Resolve is found on it.
   */
  resolveHere: boolean;
  version: string;
  /** A newer Linker-Edit on npm, or null. For `npx linker-edit`; the desktop app has appUpdate instead. */
  update: string | null;
  /** The updates of the desktop app this Linker-Edit runs in; null when it isn't one, as with `npx`. */
  appUpdate: AppUpdate | null;
  /** The thread's project folder. */
  workspaceDir: string | null;
  preview: PreviewState;
  context: ContextUsage | null;
  /** Why a message in this thread can't go to the agent right now; null when it can. */
  blocked: { message: string; canOpen: boolean } | null;
  /** Linker-Edit is opening this project in Resolve. */
  opening: string | null;
  /** Linker-Edit is making an AI timeline of this project its main one. */
  applying: string | null;
  transcription: TranscriptionStatus;
  remote: RemoteAccess;
  connect: LinkerConnect;
  /** The name of the computer this Linker-Edit runs on. */
  computer: string;
  /** Null in tests and builds that don't say. */
  setup: ComputerSetup | null;
  linker: LinkerAccount;
  /** Every workspace and series, workspaces first, each in the order it was made. */
  collections: Array<CollectionSummary>;
};

export type ClientMessage =
  | { type: "send"; text: string; compactFirst?: boolean }
  | { type: "compact" }
  | { type: "stop" }
  | { type: "set_provider"; provider: Provider }
  | { type: "set_default_provider"; provider: Provider }
  | { type: "set_approval_mode"; mode: ApprovalMode }
  | { type: "approve"; id: string; allow: boolean }
  | { type: "refresh_resolve" }
  | { type: "set_resolve_here"; resolveHere: boolean }
  | { type: "load_history" }
  | { type: "load_project_file" }
  | { type: "save_project_file"; projectId: string; text: string }
  | { type: "render_preview"; timelineId?: string }
  | { type: "apply_timeline"; projectId: string; timelineId: string }
  | { type: "stop_preview" }
  | { type: "select_thread"; threadId: string }
  | { type: "new_thread"; projectId: string }
  | { type: "rename_thread"; threadId: string; title: string }
  | { type: "archive_thread"; threadId: string; archived: boolean }
  | { type: "delete_thread"; threadId: string }
  | { type: "add_project"; source: ProjectSource }
  | {
    type: "update_project";
    projectId: string;
    name?: string;
    /** null takes the icon away, and so does an icon parseProjectIcon refuses: the project falls back to its monogram. */
    icon?: ProjectIcon | null;
    pinned?: boolean;
    archived?: boolean;
  }
  | { type: "relink_project"; projectId: string }
  | { type: "remove_project"; projectId: string }
  | { type: "open_in_resolve"; projectId: string }
  | { type: "load_library" }
  | { type: "browse_folders"; requestId: number; path: string }
  | { type: "refresh_remote_access" }
  | { type: "set_remote_access"; enabled: boolean }
  | { type: "refresh_connect" }
  | { type: "set_connect"; enabled: boolean }
  | { type: "connect_unlink"; computerId: string }
  | { type: "connect_sign_out_device"; deviceId: string }
  | { type: "linker_cancel_sign_in" }
  | { type: "linker_sign_out" }
  | { type: "transcription_setup" }
  | { type: "transcription_cancel" }
  | { type: "check_app_update" }
  | { type: "install_app_update" }
  | { type: "cancel_app_update" }
  | LibraryClientMessage
  | CollectionMessage
  | SidebarClientMessage;

/** Between the desktop app (desktop/main.ts) and the server it starts, over the child process's IPC channel. */
export type DesktopToServer =
  | { type: "update_state"; state: DesktopUpdateState }
  | { type: "install_requested" };

export type ServerToDesktop =
  | { type: "ready"; url: string }
  | { type: "check_update" }
  | { type: "install_update" };

export type ServerMessage =
  | { type: "snapshot"; state: AppState }
  | { type: "meta"; meta: AppMeta }
  | { type: "item"; threadId: string; item: ChatItem }
  | { type: "history"; projectId: string; entries: Array<HistoryEntry> }
  | { type: "project_file"; projectId: string; file: string; text: string }
  | { type: "library"; library: { error: string } | ResolveLibrary }
  | { type: "folders"; requestId: number; listing: { error: string } | FolderListing }
  | { type: "refused"; threadId: string; text: string }
  | { type: "toast"; text: string; tone: "info" | "error" }
  | LibraryServerMessage;

// --- Library ---
// Folders of files the user uses again and again (music, sound effects, logos, LUTs…). Linker-Edit only points
// at them: it never copies, moves or deletes a file in them.

export const LIBRARY_KINDS = ["music", "sfx", "graphics", "luts", "footage", "other"] as const;

export type LibraryKind = "music" | "sfx" | "graphics" | "luts" | "footage" | "other";

export type LibraryFolder = {
  id: string;
  kind: LibraryKind;
  /** "Epidemic downloads", "Brand logos". */
  label: string;
  /** This machine's path to it; kept in this machine's files, never committed. */
  path: string;
  /** "licensed for YouTube only", "use for intros". Agents read it. */
  note: string;
};

/** Which library. Most specific last in a chain: universal, then (later) workspace and series, then the project. */
export type LibraryScope =
  | { kind: "universal" }
  | { kind: "project"; projectId: string };

/** A project and timeline a library file was brought into by an agent. */
export type LibraryUse = {
  projectId: string;
  project: string;
  timeline: string | null;
  /** ISO 8601. */
  at: string;
};

export type LibraryFile = {
  folderId: string;
  scope: LibraryScope["kind"];
  kind: LibraryKind;
  /** The path inside its folder, with / between folders. Plays from /library/<folderId>/<rel>. */
  rel: string;
  name: string;
  /** Absolute, on the computer Linker-Edit runs on. */
  path: string;
  size: number;
  /** ISO 8601. */
  modified: string;
  /** Seconds; null for stills and files ffmpeg can't read. */
  duration: number | null;
  channels: number | null;
  /** Integrated loudness in LUFS; null when not measured (stills, footage, no audio). */
  loudness: number | null;
  tags: Array<string>;
  star: boolean;
  note: string;
  usedIn: Array<LibraryUse>;
};

export type LibraryFolderStatus = {
  /** Files found by the last scan. */
  files: number;
  /** While a scan runs. */
  scanning: { done: number; total: number } | null;
  /** ISO 8601; null before the first scan. */
  scannedAt: string | null;
  /** Why the last scan failed, such as a folder that's gone. */
  problem: string | null;
  id: string;
  kind: LibraryKind;
  /** "Epidemic downloads", "Brand logos". */
  label: string;
  /** This machine's path to it; kept in this machine's files, never committed. */
  path: string;
  /** "licensed for YouTube only", "use for intros". Agents read it. */
  note: string;
};

export type LibraryQuery = {
  kind?: LibraryKind;
  /** Words to find in the file's name, tags and note. */
  query?: string;
  tags?: Array<string>;
  minSeconds?: number;
  maxSeconds?: number;
  starred?: boolean;
};

/** One library as the library page shows it. */
export type LibraryView = {
  scope: LibraryScope;
  /** The libraries this page can switch between, least specific first. */
  scopes: Array<{ scope: LibraryScope; label: string }>;
  folders: Array<LibraryFolderStatus>;
  /** The first matches; total says how many there are. */
  files: Array<LibraryFile>;
  total: number;
};

export type LibraryClientMessage =
  | { type: "library_open"; scope: LibraryScope; query: LibraryQuery }
  | { type: "library_close" }
  | { type: "library_add_folder"; scope: LibraryScope; folder: Omit<LibraryFolder, "id"> }
  | {
    type: "library_update_folder";
    scope: LibraryScope;
    folderId: string;
    changes: Partial<Omit<LibraryFolder, "id">>;
  }
  | { type: "library_remove_folder"; scope: LibraryScope; folderId: string }
  | { type: "library_rescan"; folderId: string }
  | { type: "library_update_file"; folderId: string; rel: string; star?: boolean; tags?: Array<string>; note?: string };

export type LibraryServerMessage = { type: "library_view"; view: LibraryView };

// --- Workspaces & series ---

/** A workspace is a client, brand or channel; a series holds episodes. The UI words are in src/lib/labels.ts. */
export type CollectionKind = "workspace" | "series";

export type IconColor = "gray" | "red" | "orange" | "amber" | "green" | "teal" | "blue" | "violet" | "pink";

/** Like T3 Code's project icons: a lucide icon, an emoji or a few letters, on a color. */
export type CollectionIcon =
  | { kind: "lucide"; name: string; color: IconColor }
  | { kind: "emoji"; emoji: string; color: IconColor }
  | { kind: "monogram"; text: string; color: IconColor };

export type CollectionSummary = {
  id: string;
  kind: CollectionKind;
  name: string;
  /** The workspace a series is in; always null for a workspace. One level deep. */
  parentId: string | null;
  icon: CollectionIcon;
  /** The style guide for the agent, in plain words; its RULES.md. */
  rules: string;
  /** Where the rules are kept, to edit them in another editor too. */
  rulesFile: string;
  /** Projects directly in it, not those in its series. */
  projectIds: Array<string>;
  /** Its projects' agent makes the first cut without asking, and says afterwards what it decided. */
  autonomous: boolean;
  /** What feedback taught the agent, oldest first. Each was accepted by the user. */
  lessons: Array<Lesson>;
  /** The agent's notes on example videos it studied, kept as files next to the rules. */
  examples: Array<{ name: string; file: string }>;
  /** The project that holds its setup threads; null until "Set up with AI" was used. */
  setupProjectId: string | null;
  /** Approved edits of its projects, newest first, to see whether the brief works. */
  edits: Array<ApprovedEdit>;
};

/** A rule the agent learned from the user's feedback, kept with the words it came from so a wrong one is easy to spot. */
export type Lesson = {
  id: string;
  rule: string;
  /** What the user said, quoted. */
  feedback: string;
  /** ISO 8601. */
  addedAt: string;
  /** The project it was learned in. */
  project: string | null;
};

/** An AI timeline the user made the main one. */
export type ApprovedEdit = {
  /** ISO 8601. */
  at: string;
  project: string;
  /** Messages with feedback the user sent before approving; 0 when the first cut was right. */
  rounds: number;
};

export type CollectionMessage =
  | {
    type: "create_collection";
    kind: CollectionKind;
    name: string;
    parentId?: string | null;
    /** An icon that isn't one is left out, so the collection gets one from its name. */
    icon?: CollectionIcon;
  }
  | {
    type: "update_collection";
    id: string;
    name?: string;
    icon?: CollectionIcon;
    rules?: string;
    parentId?: string | null;
    autonomous?: boolean;
  }
  | { type: "remove_lesson"; id: string; lessonId: string }
  | { type: "remove_example"; id: string; name: string }
  | { type: "setup_collection"; id: string }
  | { type: "delete_collection"; id: string }
  | { type: "move_project"; projectId: string; collectionId: string | null };

// --- Sidebar (T3 Code) ---
// Pinning, settling, snoozing and unread threads, and project icons, as T3 Code's sidebar has them (MIT, see NOTICE).

/** T3 Code's project icon colors. */
export const PROJECT_ICON_COLORS = [
  "gray",
  "red",
  "orange",
  "amber",
  "yellow",
  "lime",
  "green",
  "emerald",
  "teal",
  "cyan",
  "sky",
  "blue",
  "indigo",
  "violet",
  "purple",
  "fuchsia",
  "pink",
  "rose",
] as const;

/** T3 Code's project icon colors. */
export type ProjectIconColor =
  | "gray"
  | "red"
  | "orange"
  | "amber"
  | "yellow"
  | "lime"
  | "green"
  | "emerald"
  | "teal"
  | "cyan"
  | "sky"
  | "blue"
  | "indigo"
  | "violet"
  | "purple"
  | "fuchsia"
  | "pink"
  | "rose";

/** A project's icon in the sidebar: a Lucide icon, an emoji, or one or two letters, as in T3 Code. */
export type ProjectIcon =
  | { kind: "lucide"; name: string; color: ProjectIconColor }
  | { kind: "emoji"; emoji: string }
  | { kind: "monogram"; text: string; color: ProjectIconColor };

/** Where a thread sits in the sidebar. All ISO 8601, null when it never happened; missing from older files. */
export type ThreadSidebarState = {
  createdAt: string;
  pinnedAt: string | null;
  /** Settled threads leave the project's list for the Settled shelf, and stay searchable. */
  settledAt: string | null;
  /** When it was last taken out of Settled. */
  unsettledAt: string | null;
  /** Hidden in the Snoozed shelf until then. */
  snoozedUntil: string | null;
  snoozedAt: string | null;
  /** When someone last looked at it; a turn that ended after that is unread. */
  lastReadAt: string | null;
  /** When the user last sent a message in it: threads sort by this. */
  lastSentAt: string | null;
  /** When its last turn ended. */
  turnEndedAt: string | null;
};

export type ProjectSidebarState = {
  /** Null shows a monogram made from the name. */
  icon: ProjectIcon | null;
  /** Pinned projects come first and stay listed with no active threads. */
  pinned: boolean;
  /** Archived projects go to the end, and only while they have active threads. */
  archived: boolean;
};

export type ProjectSidebarChanges = {
  /** null takes the icon away, and so does an icon parseProjectIcon refuses: the project falls back to its monogram. */
  icon?: ProjectIcon | null;
  pinned?: boolean;
  archived?: boolean;
};

export type SidebarClientMessage =
  | { type: "pin_thread"; threadId: string; pinned: boolean }
  | { type: "settle_thread"; threadId: string; settled: boolean }
  | { type: "snooze_thread"; threadId: string; until: string | null }
  | { type: "mark_read"; threadId: string; read: boolean };
