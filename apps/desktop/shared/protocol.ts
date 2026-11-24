// Messages exchanged between the local server and the browser UI over /ws.
// The iPad app (apps/ipad) reads them too: after a change, run
// RECORD_IPAD_FIXTURES=1 npm test -- scripts/ipad-fixtures.test.ts and push, so its CI decodes the new messages.

export type Provider = "claude" | "codex";

/**
 * Like T3 Code's runtime modes. "auto" (the default) is full access: everything allowed runs without asking.
 * "ask" is supervised: Resolve changes, scripts and file edits wait for the user. What the policy refuses,
 * it refuses in both.
 */
export type ApprovalMode = "ask" | "auto";

export type ToolStatus = "running" | "done" | "error";

export type ChatItem =
  | { id: string; kind: "user"; text: string }
  | { id: string; kind: "assistant"; text: string }
  | {
      id: string;
      kind: "tool";
      name: string;
      input: unknown;
      status: ToolStatus;
      output?: string;
    }
  | { id: string; kind: "notice"; text: string; tone: "info" | "error" };

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
export interface ResolveDatabase {
  /** "Disk" or "PostgreSQL" in Resolve's documentation; anything else is treated as a cloud library. */
  type: string;
  name: string;
  /** PostgreSQL server address. */
  host?: string;
}

/**
 * Which Resolve project a Linker-Edit project belongs to. Only what is the same on every machine,
 * so it can be committed and shared; see LocalLink for the rest.
 */
export interface ResolveLink {
  kind: "local" | "network" | "cloud";
  name: string;
  /** Resolve's unique project id once Linker-Edit has seen the project open; null before. */
  projectId: string | null;
}

/** This machine's side of a link: where to find the project, never committed. */
export interface LocalLink {
  database?: ResolveDatabase;
  /** Project Manager folders down to the project. */
  folder?: string[];
}

export interface ThreadSummary extends ThreadSidebarState {
  id: string;
  title: string;
  /** ISO 8601. */
  updatedAt: string;
  archived: boolean;
  busy: boolean;
  /** An approval is waiting for the user. */
  waiting: boolean;
}

export interface ProjectSummary extends ProjectSidebarState {
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
  threads: ThreadSummary[];
  /**
   * A workspace's or series' setup threads: the agent studies example videos and finished projects and writes the
   * brief here. It has no Resolve project and never gets one.
   */
  setup: boolean;
}

/** A project in the Resolve database that is open now, for adding it to Linker-Edit. */
export interface LibraryProject {
  name: string;
  folder: string[];
}

export interface ResolveLibrary {
  database: ResolveDatabase | null;
  databases: ResolveDatabase[];
  projects: LibraryProject[];
}

/** Where a new project comes from. */
export type ProjectSource =
  | { kind: "open" }
  | { kind: "library"; name: string; folder: string[]; database: ResolveDatabase | null }
  /** A new Resolve project, made in the database Resolve has open. Never in Blackmagic Cloud. */
  | { kind: "new"; name: string }
  /** A Blackmagic Cloud project by name. Linker-Edit never opens it: you open it in Resolve yourself. */
  | { kind: "cloud"; name: string }
  /** A working folder that already has a linker-edit.json, such as a teammate's cloned history. */
  | { kind: "folder"; dir: string };

/** One folder in a FolderListing. */
export interface FolderEntry {
  name: string;
  /** Holds a linker-edit.json, so it can be added as a project folder. */
  isProject: boolean;
}

/** The folders inside one folder on the server's computer, for picking a path without typing it all. */
export interface FolderListing {
  /** Absolute, ending in a separator, such as C:\Users\you\. */
  dir: string;
  isProject: boolean;
  entries: FolderEntry[];
  /**
   * The names of the files in it, shown greyed out so you can see what a folder holds before choosing it.
   * Missing from older servers.
   */
  files?: string[];
  /** The drives on Windows, or / elsewhere: where to jump from the picker. */
  roots: string[];
  home: string;
}

export interface ApprovalRequest {
  id: string;
  tool: string;
  input: unknown;
}

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
export interface TimelineStrip {
  /** Resolve's timeline id, for previewing it. */
  id: string;
  name: string;
  state: "new" | "changed";
  tiles: StripTile[];
}

/** One save in the workspace's edit history, newest first. */
export interface HistoryEntry {
  commit: string;
  /** ISO 8601. */
  date: string;
  author: string;
  /** The agent that made the change; null for the user's own changes in Resolve. */
  agent: Provider | null;
  subject: string;
  prompt: string | null;
  actions: string[];
  changes: string[];
  /** The Linker-Edit thread the agent's turn ran in; null for other saves. */
  thread: string | null;
  timelines: TimelineStrip[];
}

export type MediaKind = "video" | "image" | "audio" | "unsupported" | "missing";

/** A clip as the quick preview plays it. Times are seconds from the timeline's start. */
export interface PlayableClip {
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
}

/** A timeline the browser can play straight from its media, without rendering. */
export interface PlayableTimeline {
  id: string;
  name: string;
  /** The save it comes from; null for the timeline as it is in Resolve now. */
  commit: string | null;
  fps: number;
  width: number;
  height: number;
  duration: number;
  video: PlayableClip[];
  audio: PlayableClip[];
  markers: { time: number; color: string; name: string }[];
  /** For an AI timeline as Resolve has it now: the main timeline it's a new version of, which it can replace. */
  replaces?: { id: string; name: string };
}

export interface PreviewTimeline {
  id: string;
  name: string;
}

/** A small render of one timeline, made by Resolve and played in the preview panel. */
export type PreviewState =
  | { state: "idle" }
  /** The timeline is null until Resolve picks up the job. */
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
export interface ContextUsage {
  usedTokens: number;
  /** Null until the turn's result reports the model's window. */
  maxTokens: number | null;
  /** When Claude's prompt cache expires; a turn after that rereads the whole chat uncached. ISO 8601. */
  cacheExpiresAt: string | null;
}

/** Below this, rereading a chat after its cache expired is cheap, so compacting isn't suggested. As in T3 Code. */
export const COMPACT_SUGGESTION_MIN_TOKENS = 200_000;

/** Speech-to-text work on this machine. Percent is null while unpacking; Whisper itself reports no progress. */
export type TranscriptionStatus =
  /** ready: whether the speech engine is downloaded; left out while unknown. */
  | { state: "idle"; ready?: boolean }
  | { state: "preparing"; step: string; percent: number | null }
  | { state: "transcribing"; file: string; done: number; total: number };

/** Opening this app from the user's other devices through Tailscale Serve. */
export type RemoteAccess =
  | { state: "checking" }
  /** Tailscale isn't installed, running or logged in here; hint says what to do. */
  | { state: "unavailable"; hint: string }
  /** problem is why turning it on failed or can't work yet, which may include a link. */
  | { state: "off"; url: string; problem: string | null }
  | { state: "switching"; url: string }
  /** Only devices logged in to Tailscale as owner get in. */
  | { state: "on"; url: string; owner: string };

/** The Linker account (linker.cool) signed in on this computer. Optional: Linker-Edit works without one. */
export type LinkerAccount =
  /** problem is why the last sign-in failed or ended, worded for the user. */
  | { state: "signed_out"; problem: string | null }
  /** Waiting for the browser to come back from Linker. */
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

/**
 * What GET /api/hello answers, which is how a page or another computer recognizes Linker-Edit. protocol is raised
 * only for a change that breaks a page of another version talking to this server (your own web address).
 */
export interface Hello {
  app: "linker-edit";
  name: string;
  version: string;
  protocol: number;
}

export const PROTOCOL_VERSION = 1;

/** Another of the user's computers that runs Linker-Edit with remote access on. */
export interface Computer {
  name: string;
  url: string;
  version: string;
  /** How it's reached: Tailscale (the default for older servers) or Linker Connect. */
  via?: "tailscale" | "linker";
  /** False for a Linker Connect computer that isn't reachable now. */
  online?: boolean;
}

/** One of the edit computers linked to your Linker account (at most LinkerConnect.limit). */
export interface LinkedComputer {
  id: string;
  name: string;
  /** https://edit-….linker.cool; null for a computer linked for Tailscale only, without a tunnel. */
  url: string | null;
  /** Its Tailscale remote access address (https://studio.tail1234.ts.net:8443) while that's on; null otherwise. */
  tailnetUrl: string | null;
  /** Whether its tunnel is connected now; null when Linker couldn't tell. */
  online: boolean | null;
  linkedAt: string;
  lastSeenAt: string | null;
}

/**
 * How a Linker Connect device stays signed in: Linker-Edit's window with a cookie, or a native app (the iPad app)
 * with a Bearer token.
 */
export type ConnectDeviceKind = "cookie" | "native";

/** A device that opened this computer through Linker Connect, and can be signed out. */
export interface ConnectDevice {
  id: string;
  name: string;
  kind: ConnectDeviceKind;
  createdAt: string;
  lastSeenAt: string;
}

/**
 * Linker Connect, like T3 Connect: this computer gets its own address at Linker, through a Cloudflare Tunnel,
 * and your other computers signed in to the same Linker account open it from anywhere.
 */
export interface LinkerConnect {
  /**
   * signed_out: sign in with Linker first. off. starting: linking, downloading cloudflared or connecting.
   * on: linked, with the tunnel running.
   */
  state: "signed_out" | "off" | "starting" | "on";
  /** "Use the tunnel": off links this computer for Tailscale only, with no address at Linker. */
  tunnel: boolean;
  /** This computer's address once linked with the tunnel. */
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
  computers: LinkedComputer[] | null;
  /** How many computers one Linker account can link. */
  limit: number;
  /** Linking stopped because the limit is reached: remove one of computers first. */
  atLimit: boolean;
  /** Devices signed in to this computer through Linker Connect. */
  devices: ConnectDevice[];
}

/**
 * One of the user's computers in the desktop app's sidebar, like a T3 Code environment: its projects show next to
 * this computer's, and their threads run there. GET /api/machines lists them, this computer first.
 */
export interface Machine {
  /** "this" for this computer; a Linker Connect computer's id; "tailscale-<host>" for one found over Tailscale only. */
  id: string;
  name: string;
  /** Its Linker-Edit's version; "" when it didn't answer. */
  version: string;
  /** How this computer reaches it. */
  via: "this" | "linker" | "tailscale";
  /** Whether it answered just now. */
  online: boolean;
  /**
   * Where the page reaches its server, as a path on this computer's server: "" for this computer, "/machines/<id>"
   * for the others, which this computer's server passes on with its own sign-in (server/machines.ts).
   */
  base: string;
}

/** GET /api/machines. */
export interface Machines {
  machines: Machine[];
}

/** GET /api/computers: this computer's name, and the user's other computers found over Tailscale. */
export interface Computers {
  name: string;
  /** This computer's Linker-Edit; missing from older ones. */
  version?: string;
  others: Computer[];
  /** Added by the desktop app: this computer's own Linker-Edit address. */
  home?: string;
}

/**
 * What the first desktop apps told their pages about an update; the preload still offers it to pages of older
 * Linker-Edits on the user's other computers. "ready" installs on restart; "download" means download it by hand.
 */
export type DesktopUpdate = { version: string; state: "ready" } | { version: string; state: "download"; url: string };

/**
 * Where the installed desktop app is with its own updates. desktop/update-state.ts moves it from one status to the
 * next; the app's server passes it on to every page, so another of the user's computers shows its own.
 */
export interface DesktopUpdateState {
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
  notes: string[];
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
}

/** AppMeta.appUpdate: the desktop app's update state, plus what this Linker-Edit is doing about it. */
export interface AppUpdate extends DesktopUpdateState {
  /** The user asked to restart to update while Linker-Edit was busy with Resolve; it restarts once it's done. */
  waiting: boolean;
}

/** Where this computer's Linker-Edit keeps things, for the settings page. Set in linker-edit.config.json. */
export interface ComputerSetup {
  /** The config file the paths come from; it may not exist yet. */
  configFile: string;
  folders: { projects: string; previews: string; transcripts: string; tools: string };
  /** Resolve's own MCP server program, found or configured; null when this computer has none. */
  resolveMcp: string | null;
}

export interface AppMeta {
  projects: ProjectSummary[];
  /** The thread this client is looking at; null when there are no projects. */
  threadId: string | null;
  projectId: string | null;
  provider: Provider;
  /** The agent new threads start with. */
  defaultProvider: Provider;
  approvalMode: ApprovalMode;
  /** This computer's threads settle by themselves after this many days without activity; null never. */
  autoSettleAfterDays: number | null;
  /** This thread's turn is running. */
  busy: boolean;
  approvals: ApprovalRequest[];
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
  blocked: {
    message: string;
    canOpen: boolean;
    /** A message goes anyway: it opens the thread's project in Resolve first. */
    canSend?: boolean;
  } | null;
  /** The Resolve project a message in this thread's project made Resolve leave, to open again; null when none. */
  reopen: string | null;
  /** Add project can make a new one in Resolve. Missing from older servers. */
  createsProjects?: boolean;
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
  collections: CollectionSummary[];
}

export interface AppState extends AppMeta {
  items: ChatItem[];
}

export type ClientMessage =
  | LibraryClientMessage
  /** compactFirst summarizes the chat before sending: for coming back after the prompt cache expired. */
  | { type: "send"; text: string; compactFirst?: boolean }
  /** Summarizes the chat so far to free context. Claude only. */
  | { type: "compact" }
  | { type: "stop" }
  | { type: "set_provider"; provider: Provider }
  /** The agent new threads start with, without changing the open thread's. */
  | { type: "set_default_provider"; provider: Provider }
  | { type: "set_approval_mode"; mode: ApprovalMode }
  /** Days between AUTO_SETTLE_DAYS.min and max, or null to never settle threads by themselves. */
  | { type: "set_auto_settle"; days: number | null }
  | { type: "approve"; id: string; allow: boolean }
  | { type: "refresh_resolve" }
  /** Opens AppMeta.reopen in Resolve again. */
  | { type: "reopen_previous" }
  /** Whether this computer has Resolve (or will); see AppMeta.resolveHere. */
  | { type: "set_resolve_here"; resolveHere: boolean }
  | { type: "load_history" }
  /** PROJECT.md of the project in view: what the video is, kept by the agent and edited here by the user. */
  | { type: "load_project_file" }
  | { type: "save_project_file"; projectId: string; text: string }
  /** Without a timeline id, renders Resolve's current timeline. */
  | { type: "render_preview"; timelineId?: string }
  /** The user is happy with an AI timeline: it takes the main timeline's place, which is kept renamed. */
  | { type: "apply_timeline"; projectId: string; timelineId: string }
  | { type: "stop_preview" }
  | { type: "select_thread"; threadId: string }
  | { type: "new_thread"; projectId: string }
  | { type: "rename_thread"; threadId: string; title: string }
  | { type: "archive_thread"; threadId: string; archived: boolean }
  | { type: "delete_thread"; threadId: string }
  | { type: "add_project"; source: ProjectSource }
  | ({ type: "update_project"; projectId: string; name?: string } & ProjectSidebarChanges)
  /** Links the project to the one open in Resolve now. */
  | { type: "relink_project"; projectId: string }
  /** Takes the project out of Linker-Edit. Its folder and its Resolve project stay. */
  | { type: "remove_project"; projectId: string }
  /** Saves the project open in Resolve, then opens this one. */
  | { type: "open_in_resolve"; projectId: string }
  | { type: "load_library" }
  /** Lists the folders in path on the server's computer; an empty path is the home folder. */
  | { type: "browse_folders"; requestId: number; path: string }
  | { type: "refresh_remote_access" }
  | { type: "set_remote_access"; enabled: boolean }
  /** Looks up the computers linked to your Linker account again. */
  | { type: "refresh_connect" }
  | { type: "set_connect"; enabled: boolean }
  /** "Use the tunnel" in Linker Connect; off links this computer for Tailscale only. */
  | { type: "set_connect_tunnel"; tunnel: boolean }
  /** Unlinks one of your computers from Linker Connect, this one or another, which frees its place. */
  | { type: "connect_unlink"; computerId: string }
  /** Signs a device out of this computer. */
  | { type: "connect_sign_out_device"; deviceId: string }
  /** Stops waiting for a Linker sign-in started in the browser (GET /api/linker/sign-in starts one). */
  | { type: "linker_cancel_sign_in" }
  | { type: "linker_sign_out" }
  /** Downloads the speech engine now, or stops that download. */
  | { type: "transcription_setup" }
  | { type: "transcription_cancel" }
  /** Looks for a new release of the desktop app now. */
  | { type: "check_app_update" }
  /** Restarts the desktop app into the downloaded release, once nothing is busy with Resolve. */
  | { type: "install_app_update" }
  /** Doesn't restart after all when Linker-Edit is done. */
  | { type: "cancel_app_update" }
  | CollectionMessage
  | SidebarClientMessage;

/** Between the desktop app (desktop/main.ts) and the server it starts, over the child process's IPC channel. */
export type DesktopToServer =
  | { type: "update_state"; state: DesktopUpdateState }
  /** A page of an older Linker-Edit asked to restart to update; the server waits until Resolve is free. */
  | { type: "install_requested" };
export type ServerToDesktop =
  | { type: "ready"; url: string }
  | { type: "check_update" }
  /** Nothing is busy with Resolve: install now. */
  | { type: "install_update" };

export type ServerMessage =
  | LibraryServerMessage
  | { type: "snapshot"; state: AppState }
  | { type: "meta"; meta: AppMeta }
  /** Only the meta fields that changed, for clients that asked for patches (/ws?meta=patch). */
  | { type: "meta_patch"; meta: Partial<AppMeta> }
  | { type: "item"; threadId: string; item: ChatItem }
  | { type: "history"; projectId: string; entries: HistoryEntry[] }
  /** A project's PROJECT.md: on request, after a save, and after each turn in the project. file is its path. */
  | { type: "project_file"; projectId: string; file: string; text: string }
  | { type: "library"; library: ResolveLibrary | { error: string } }
  | { type: "folders"; requestId: number; listing: FolderListing | { error: string } }
  /** The message wasn't sent; it goes back in the composer. */
  | { type: "refused"; threadId: string; text: string }
  | { type: "toast"; text: string; tone: "info" | "error" };

// --- Library ---
// Folders of files the user uses again and again (music, sound effects, logos, LUTs…). Linker-Edit only points
// at them: it never copies, moves or deletes a file in them.

export const LIBRARY_KINDS = ["music", "sfx", "graphics", "luts", "footage", "other"] as const;
export type LibraryKind = (typeof LIBRARY_KINDS)[number];

export interface LibraryFolder {
  id: string;
  kind: LibraryKind;
  /** "Epidemic downloads", "Brand logos". */
  label: string;
  /** This machine's path to it; kept in this machine's files, never committed. */
  path: string;
  /** "licensed for YouTube only", "use for intros". Agents read it. */
  note: string;
}

/** Which library. Most specific last in a chain: universal, then (later) workspace and series, then the project. */
export type LibraryScope = { kind: "universal" } | { kind: "project"; projectId: string };

/** A project and timeline a library file was brought into by an agent. */
export interface LibraryUse {
  projectId: string;
  project: string;
  timeline: string | null;
  /** ISO 8601. */
  at: string;
}

export interface LibraryFile {
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
  tags: string[];
  star: boolean;
  note: string;
  usedIn: LibraryUse[];
}

export interface LibraryFolderStatus extends LibraryFolder {
  /** Files found by the last scan. */
  files: number;
  /** While a scan runs. */
  scanning: { done: number; total: number } | null;
  /** ISO 8601; null before the first scan. */
  scannedAt: string | null;
  /** Why the last scan failed, such as a folder that's gone. */
  problem: string | null;
}

export interface LibraryQuery {
  kind?: LibraryKind;
  /** Words to find in the file's name, tags and note. */
  query?: string;
  tags?: string[];
  minSeconds?: number;
  maxSeconds?: number;
  starred?: boolean;
}

/** One library as the library page shows it. */
export interface LibraryView {
  scope: LibraryScope;
  /** The libraries this page can switch between, least specific first. */
  scopes: { scope: LibraryScope; label: string }[];
  folders: LibraryFolderStatus[];
  /** The first matches; total says how many there are. */
  files: LibraryFile[];
  total: number;
}

/** How many of each kind of file a folder holds, counted up to a few thousand. */
export interface LibraryFileCounts {
  audio: number;
  video: number;
  image: number;
  lut: number;
  other: number;
}

/** One folder inside an assets folder, with the kind Linker-Edit guessed for it. */
export interface LibrarySortedFolder {
  /** Its path inside the assets folder, with / between folders: "Music", or "Audio/SFX". */
  name: string;
  path: string;
  /** Null when neither its name nor its files say. */
  kind: LibraryKind | null;
  /** What the guess is from. */
  reason: "name" | "files" | null;
  counts: LibraryFileCounts;
  /** It holds more files than were counted. */
  more: boolean;
}

/** An assets folder such as video-editing/ sorted into library folders, for the user to check before adding. */
export interface LibrarySort {
  root: string;
  /** The kind the assets folder's own name says, such as for Music/ with only Epidemic/ and Artlist/ in it. */
  rootKind: LibraryKind | null;
  folders: LibrarySortedFolder[];
  /** More folders than are listed. */
  more: boolean;
}

export type LibraryClientMessage =
  /** Shows a library, and keeps sending it while it changes, until library_close. */
  | { type: "library_open"; scope: LibraryScope; query: LibraryQuery }
  | { type: "library_close" }
  | { type: "library_add_folder"; scope: LibraryScope; folder: Omit<LibraryFolder, "id"> }
  /** Several at once, such as the folders of an assets folder; any that can't be added are said in a toast. */
  | { type: "library_add_folders"; scope: LibraryScope; folders: Omit<LibraryFolder, "id">[] }
  /** Guesses the kind of each folder inside path; the answer is a library_sorted with the same requestId. */
  | { type: "library_sort_folder"; requestId: number; path: string }
  | { type: "library_update_folder"; scope: LibraryScope; folderId: string; changes: Partial<Omit<LibraryFolder, "id">> }
  /** Forgets the folder. The folder and its files stay where they are. */
  | { type: "library_remove_folder"; scope: LibraryScope; folderId: string }
  | { type: "library_rescan"; folderId: string }
  | { type: "library_update_file"; folderId: string; rel: string; star?: boolean; tags?: string[]; note?: string };

export type LibraryServerMessage =
  | { type: "library_view"; view: LibraryView }
  | { type: "library_sorted"; requestId: number; sort: LibrarySort | { error: string } };

// --- Workspaces & series ---

/** A workspace is a client, brand or channel; a series holds episodes. The UI words are in src/lib/labels.ts. */
export type CollectionKind = "workspace" | "series";

export type IconColor = "gray" | "red" | "orange" | "amber" | "green" | "teal" | "blue" | "violet" | "pink";

/** Like T3 Code's project icons: a lucide icon, an emoji or a few letters, on a color. */
export type CollectionIcon =
  /** name is the icon's name in lucide, such as "mic" or "clapperboard". */
  | { kind: "lucide"; name: string; color: IconColor }
  | { kind: "emoji"; emoji: string; color: IconColor }
  | { kind: "monogram"; text: string; color: IconColor };

export interface CollectionSummary {
  id: string;
  kind: CollectionKind;
  name: string;
  /** The workspace a series is in; always null for a workspace. One level deep. */
  parentId: string | null;
  icon: CollectionIcon;
  /** The style guide for the agent, in plain words; its SKILL.md without the front matter. */
  rules: string;
  /** Where the rules are kept, to edit them in another editor too. */
  rulesFile: string;
  /** Projects directly in it, not those in its series. */
  projectIds: string[];
  /** Its projects' agent makes the first cut without asking, and says afterwards what it decided. */
  autonomous: boolean;
  /** What feedback taught the agent, oldest first. Each was accepted by the user. */
  lessons: Lesson[];
  /** The agent's notes on example videos it studied, kept as files next to the rules. */
  examples: { name: string; file: string }[];
  /** The project that holds its setup threads; null until "Set up with AI" was used. */
  setupProjectId: string | null;
  /** Approved edits of its projects, newest first, to see whether the brief works. */
  edits: ApprovedEdit[];
}

/** A rule the agent learned from the user's feedback, kept with the words it came from so a wrong one is easy to spot. */
export interface Lesson {
  id: string;
  rule: string;
  /** What the user said, quoted. */
  feedback: string;
  /** ISO 8601. */
  addedAt: string;
  /** The project it was learned in. */
  project: string | null;
}

/** An AI timeline the user made the main one. */
export interface ApprovedEdit {
  /** ISO 8601. */
  at: string;
  project: string;
  /** Messages with feedback the user sent before approving; 0 when the first cut was right. */
  rounds: number;
}

export type CollectionMessage =
  /** A series may go in a workspace; a workspace goes in nothing. */
  | { type: "create_collection"; kind: CollectionKind; name: string; parentId?: string | null; icon?: CollectionIcon }
  | { type: "update_collection"; id: string; name?: string; icon?: CollectionIcon; rules?: string; parentId?: string | null; autonomous?: boolean }
  /** Forgets a lesson or the notes on an example. */
  | { type: "remove_lesson"; id: string; lessonId: string }
  | { type: "remove_example"; id: string; name: string }
  /** Opens its setup thread, where the agent studies example videos and writes the brief; made on first use. */
  | { type: "setup_collection"; id: string }
  /** Refused while projects or series are in it. */
  | { type: "delete_collection"; id: string }
  /** Puts a project in a workspace or series, or in none. */
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

export type ProjectIconColor = (typeof PROJECT_ICON_COLORS)[number];

/** A project's icon in the sidebar: a Lucide icon, an emoji, or one or two letters, as in T3 Code. */
export type ProjectIcon =
  | { kind: "lucide"; name: string; color: ProjectIconColor }
  | { kind: "emoji"; emoji: string }
  | { kind: "monogram"; text: string; color: ProjectIconColor };

/** Where a thread sits in the sidebar. All ISO 8601, null when it never happened; missing from older files. */
export interface ThreadSidebarState {
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
}

export interface ProjectSidebarState {
  /** Null shows a monogram made from the name. */
  icon: ProjectIcon | null;
  /** Pinned projects come first and stay listed with no active threads. */
  pinned: boolean;
  /** Archived projects go to the end, and only while they have active threads. */
  archived: boolean;
}

export interface ProjectSidebarChanges {
  icon?: ProjectIcon | null;
  pinned?: boolean;
  archived?: boolean;
}

export type SidebarClientMessage =
  | { type: "pin_thread"; threadId: string; pinned: boolean }
  /** Settling also unpins and wakes it, like T3 Code. */
  | { type: "settle_thread"; threadId: string; settled: boolean }
  /** A null until wakes it now. */
  | { type: "snooze_thread"; threadId: string; until: string | null }
  /** read: false marks the last turn unread again. */
  | { type: "mark_read"; threadId: string; read: boolean };

/** Quiet threads settle by themselves after this many days, as in T3 Code; AppMeta.autoSettleAfterDays. */
export const AUTO_SETTLE_DAYS = { default: 7, min: 1, max: 90 } as const;
