import type { ChatItem, ToolStatus } from "../../shared/protocol.ts";

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
}

export interface ProviderSession {
  send(text: string, sink: TurnSink, signal: AbortSignal): Promise<void>;
}

export function truncate(text: string, max = 4000): string {
  return text.length > max ? `${text.slice(0, max)}\n… (${text.length - max} more characters)` : text;
}
