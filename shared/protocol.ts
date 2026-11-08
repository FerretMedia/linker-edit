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
      timeline: string | null;
    }
  | { state: "not_running" }
  | { state: "unreachable"; hint: string }
  | { state: "blocked"; hint: string }
  | { state: "setup_needed"; hint: string };

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

export interface AppMeta {
  provider: Provider;
  approvalMode: ApprovalMode;
  busy: boolean;
  approvals: ApprovalRequest[];
  resolve: ResolveStatus;
  workspaceDir: string;
  preview: PreviewState;
}

export interface AppState extends AppMeta {
  items: ChatItem[];
}

export type ClientMessage =
  | { type: "send"; text: string }
  | { type: "stop" }
  | { type: "new_chat" }
  | { type: "set_provider"; provider: Provider }
  | { type: "set_approval_mode"; mode: ApprovalMode }
  | { type: "approve"; id: string; allow: boolean }
  | { type: "refresh_resolve" }
  | { type: "load_history" }
  /** Without a timeline id, renders Resolve's current timeline. */
  | { type: "render_preview"; timelineId?: string }
  | { type: "stop_preview" };

export type ServerMessage =
  | { type: "snapshot"; state: AppState }
  | { type: "meta"; meta: AppMeta }
  | { type: "item"; item: ChatItem }
  | { type: "history"; entries: HistoryEntry[] };
