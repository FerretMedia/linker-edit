import type { ClipChange, CloudSyncMode, Provider, ResolveLink } from "#shared/protocol.ts";

const RESOLVE_PREFIX = "mcp__resolve__";

export const PROVIDER_LABEL: Record<Provider, string> = { claude: "Claude", codex: "Codex" };

export const isResolveTool = (name: string) => name.startsWith(RESOLVE_PREFIX);

/** "mcp__resolve__timeline" + {action: "list"} → "timeline.list"; other tools keep their name. */
export function toolLabel(name: string, input: unknown): string {
  if (!isResolveTool(name)) return name;
  const action = (input as { action?: unknown } | null)?.action;
  return `${name.slice(RESOLVE_PREFIX.length)}${typeof action === "string" ? `.${action}` : ""}`;
}

/** The one-line detail shown after a tool's label. */
export function toolSummary(name: string, input: unknown): string {
  if (input === null || typeof input !== "object") return "";
  const record = input as Record<string, unknown>;
  if (isResolveTool(name)) {
    return record.params && typeof record.params === "object" ? compactJson(record.params) : "";
  }
  for (const key of ["command", "file_path", "path", "pattern", "query", "url", "description"]) {
    if (typeof record[key] === "string") return record[key];
  }
  return "";
}

function compactJson(value: unknown): string {
  const text = JSON.stringify(value);
  return text === "{}" ? "" : text;
}

export const formatJson = (value: unknown) => (typeof value === "string" ? value : JSON.stringify(value, null, 2));

/** `"AI - Cut": 2 clips added` → the timeline and what happened to it, as written by describeChanges. */
export function splitChange(change: string): { timeline: string | null; detail: string } {
  const match = /^"(.*)": (.*)$/.exec(change);
  return match ? { timeline: match[1]!, detail: match[2]! } : { timeline: null, detail: change };
}

const dayFormat = new Intl.DateTimeFormat(undefined, { weekday: "short", month: "short", day: "numeric" });

export function dayLabel(date: Date, now: Date): string {
  const startOfDay = (value: Date) => new Date(value.getFullYear(), value.getMonth(), value.getDate()).getTime();
  const days = Math.round((startOfDay(now) - startOfDay(date)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  return dayFormat.format(date);
}

/** Colours for clip changes on the history strips; the legend reads the same map. */
export const CHANGE_STYLE: Record<ClipChange, { ring: string; dot: string }> = {
  added: { ring: "ring-success", dot: "bg-success" },
  moved: { ring: "ring-primary", dot: "bg-primary" },
  trimmed: { ring: "ring-warning", dot: "bg-warning" },
  changed: { ring: "ring-violet-500", dot: "bg-violet-500" },
  removed: { ring: "ring-destructive", dot: "bg-destructive" },
};

export const LIBRARY_LABEL: Record<ResolveLink["kind"], string> = {
  local: "Local database",
  network: "Network database",
  cloud: "Blackmagic Cloud",
};

/** How much of a cloud project's media Resolve downloads to this computer. */
export const SYNC_MODES: { value: CloudSyncMode; label: string }[] = [
  { value: "proxy_only", label: "Proxies only" },
  { value: "proxy_and_orig", label: "Proxies and original media" },
  { value: "none", label: "No media" },
];

/** "5m", "3h", "2d" ago, for thread rows; recomputed when the list changes, not on a timer. */
export function ago(date: Date, now: Date): string {
  const minutes = Math.round((now.getTime() - date.getTime()) / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.round(minutes / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.round(hours / 24);
  return days < 7 ? `${days}d` : `${Math.round(days / 7)}w`;
}
