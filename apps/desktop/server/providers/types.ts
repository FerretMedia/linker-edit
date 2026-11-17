import type { ChatItem, ContextUsage, ToolStatus } from "../../shared/protocol.ts";

/** Partial update for a chat item; the server merges it into the stored item by id. */
export interface ItemPatch {
  id: string;
  kind?: ChatItem["kind"];
  text?: string;
  tone?: "info" | "error";
  name?: string;
  input?: unknown;
  status?: ToolStatus;
  output?: string;
}

export interface TurnSink {
  upsert(patch: ItemPatch): void;
  notice(text: string, tone?: "info" | "error"): void;
  /** Resolves true when the user allows the call. */
  requestApproval(tool: string, input: unknown): Promise<boolean>;
  /** Null when the size is no longer known, as after a compaction that didn't report it. */
  context(usage: ContextUsage | null): void;
  /**
   * Backs the open Resolve project up, once per turn, before the first change to it. Rejects when that
   * fails, and the change must not run then. Absent where there is no project to back up.
   */
  backup?(): Promise<void>;
}

export interface ProviderSession {
  send(text: string, sink: TurnSink, signal: AbortSignal): Promise<void>;
  /** Summarizes the conversation so far to free context. Absent where the provider can't. */
  compact?(sink: TurnSink, signal: AbortSignal): Promise<void>;
  /** The provider's id for this conversation, to continue it after a restart; null before the first turn. */
  resumeId(): string | null;
}

export function truncate(text: string, max = 4000): string {
  return text.length > max ? `${text.slice(0, max)}\n… (${text.length - max} more characters)` : text;
}
