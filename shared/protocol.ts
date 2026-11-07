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

export interface AppMeta {
  provider: Provider;
  approvalMode: ApprovalMode;
  busy: boolean;
  approvals: ApprovalRequest[];
  resolve: ResolveStatus;
  workspaceDir: string;
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
  | { type: "load_history" };

export type ServerMessage =
  | { type: "snapshot"; state: AppState }
  | { type: "meta"; meta: AppMeta }
  | { type: "item"; item: ChatItem }
  | { type: "history"; entries: HistoryEntry[] };
