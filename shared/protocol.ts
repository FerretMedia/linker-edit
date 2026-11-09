// Messages exchanged between the local server and the browser UI over /ws.

export type Provider = "claude" | "codex";

/** "ask": Resolve writes and file edits wait for the user. "auto": everything runs. */
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

export type CloudSyncMode = "none" | "proxy_only" | "proxy_and_orig";

/** This machine's side of a link: where to find the project, never committed. */
export interface LocalLink {
  database?: ResolveDatabase;
  /** Project Manager folders down to the project. */
  folder?: string[];
  /** Where a cloud project's media goes on this machine. Resolve needs one to open it. */
  mediaPath?: string;
  syncMode?: CloudSyncMode;
}

export interface ThreadSummary {
  id: string;
  title: string;
  /** ISO 8601. */
  updatedAt: string;
  archived: boolean;
  busy: boolean;
  /** An approval is waiting for the user. */
  waiting: boolean;
}

export interface ProjectSummary {
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
  | { kind: "cloud"; name: string; mediaPath: string; syncMode: CloudSyncMode }
  /** A working folder that already has a linker-edit.json, such as a teammate's cloned history. */
  | { kind: "folder"; dir: string };

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

export interface AppMeta {
  projects: ProjectSummary[];
  /** The thread this client is looking at; null when there are no projects. */
  threadId: string | null;
  projectId: string | null;
  provider: Provider;
  approvalMode: ApprovalMode;
  /** This thread's turn is running. */
  busy: boolean;
  approvals: ApprovalRequest[];
  resolve: ResolveStatus;
  /** The thread's project folder. */
  workspaceDir: string | null;
  preview: PreviewState;
  context: ContextUsage | null;
  /** Why a message in this thread can't go to the agent right now; null when it can. */
  blocked: { message: string; canOpen: boolean } | null;
  /** Linker-Edit is opening this project in Resolve. */
  opening: string | null;
}

export interface AppState extends AppMeta {
  items: ChatItem[];
}

export type ClientMessage =
  /** compactFirst summarizes the chat before sending: for coming back after the prompt cache expired. */
  | { type: "send"; text: string; compactFirst?: boolean }
  /** Summarizes the chat so far to free context. Claude only. */
  | { type: "compact" }
  | { type: "stop" }
  | { type: "set_provider"; provider: Provider }
  | { type: "set_approval_mode"; mode: ApprovalMode }
  | { type: "approve"; id: string; allow: boolean }
  | { type: "refresh_resolve" }
  | { type: "load_history" }
  /** Without a timeline id, renders Resolve's current timeline. */
  | { type: "render_preview"; timelineId?: string }
  | { type: "stop_preview" }
  | { type: "select_thread"; threadId: string }
  | { type: "new_thread"; projectId: string }
  | { type: "rename_thread"; threadId: string; title: string }
  | { type: "archive_thread"; threadId: string; archived: boolean }
  | { type: "delete_thread"; threadId: string }
  | { type: "add_project"; source: ProjectSource }
  | { type: "update_project"; projectId: string; name?: string; mediaPath?: string; syncMode?: CloudSyncMode }
  /** Links the project to the one open in Resolve now. */
  | { type: "relink_project"; projectId: string }
  /** Takes the project out of Linker-Edit. Its folder and its Resolve project stay. */
  | { type: "remove_project"; projectId: string }
  /** Saves the project open in Resolve, then opens this one. */
  | { type: "open_in_resolve"; projectId: string }
  | { type: "load_library" };

export type ServerMessage =
  | { type: "snapshot"; state: AppState }
  | { type: "meta"; meta: AppMeta }
  | { type: "item"; threadId: string; item: ChatItem }
  | { type: "history"; projectId: string; entries: HistoryEntry[] }
  | { type: "library"; library: ResolveLibrary | { error: string } }
  /** The message wasn't sent; it goes back in the composer. */
  | { type: "refused"; threadId: string; text: string }
  | { type: "toast"; text: string; tone: "info" | "error" };
