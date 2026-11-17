import type { ClipChange, CloudSyncMode, CollectionKind, Provider, ResolveLink } from "#shared/protocol.ts";

// Resolve's own MCP server, and Linker-Edit's, whose tools work in Resolve except the ones named here.
const RESOLVE_PREFIX = "mcp__resolve__";
const LINKER_PREFIX = "mcp__linker__";
const NOT_RESOLVE = new Set(["transcribe", "study_media", "compare_cut", "save_brief", "add_lesson", "save_example"]);

export const PROVIDER_LABEL: Record<Provider, string> = { claude: "Claude", codex: "Codex" };

export const isResolveTool = (name: string) =>
  name.startsWith(RESOLVE_PREFIX) || (name.startsWith(LINKER_PREFIX) && !NOT_RESOLVE.has(name.slice(LINKER_PREFIX.length)));

/** A change to a workspace's or series' brief that an agent proposes; the user accepts or declines it. */
export type BriefRequest =
  | { kind: "rules"; for: string; summary: string; was: string; rules: string }
  | { kind: "lesson"; for: string; feedback: string; rule: string };

/** What an approval of save_brief or add_lesson shows; null for every other tool. */
export function briefRequest(name: string, input: unknown): BriefRequest | null {
  const shown = (input ?? {}) as Record<string, unknown>;
  const text = (key: string) => (typeof shown[key] === "string" ? shown[key] : "");
  if (name === `${LINKER_PREFIX}save_brief`) return { kind: "rules", for: text("for"), summary: text("summary"), was: text("was"), rules: text("rules") };
  if (name === `${LINKER_PREFIX}add_lesson`) return { kind: "lesson", for: text("for"), feedback: text("feedback"), rule: text("rule") };
  return null;
}

/** The lines of a text change, in order: kept, taken out or added. For showing new rules against the old. */
export function lineDiff(before: string, after: string): { kind: "same" | "removed" | "added"; text: string }[] {
  const a = before.trim() ? before.trim().split("\n") : [];
  const b = after.trim() ? after.trim().split("\n") : [];
  // The longest run of lines both have, from the end, so walking forward gives the changes in order.
  const common = Array.from({ length: a.length + 1 }, () => new Array<number>(b.length + 1).fill(0));
  for (let i = a.length - 1; i >= 0; i -= 1) {
    for (let j = b.length - 1; j >= 0; j -= 1) common[i]![j] = a[i] === b[j] ? common[i + 1]![j + 1]! + 1 : Math.max(common[i + 1]![j]!, common[i]![j + 1]!);
  }
  const lines: { kind: "same" | "removed" | "added"; text: string }[] = [];
  let i = 0;
  let j = 0;
  while (i < a.length || j < b.length) {
    if (i < a.length && j < b.length && a[i] === b[j]) {
      lines.push({ kind: "same", text: a[i]! });
      i += 1;
      j += 1;
    } else if (i < a.length && (j === b.length || common[i + 1]![j]! >= common[i]![j + 1]!)) {
      lines.push({ kind: "removed", text: a[i]! });
      i += 1;
    } else {
      lines.push({ kind: "added", text: b[j]! });
      j += 1;
    }
  }
  return lines;
}

const isScript = (name: string, input: unknown): input is { script: string } =>
  name === `${RESOLVE_PREFIX}run_script` && typeof (input as { script?: unknown } | null)?.script === "string";

/** "mcp__linker__create_timeline" → "create_timeline"; other tools keep their name. */
export function toolLabel(name: string, _input: unknown): string {
  return name.startsWith(RESOLVE_PREFIX) || name.startsWith(LINKER_PREFIX) ? name.slice(name.indexOf("__", 5) + 2) : name;
}

/** The one-line detail shown after a tool's label. */
export function toolSummary(name: string, input: unknown): string {
  if (input === null || typeof input !== "object") return "";
  if (isScript(name, input)) {
    return input.script.split("\n").find((line) => line.trim() && !line.trim().startsWith("#"))?.trim() ?? "";
  }
  const record = input as Record<string, unknown>;
  if (isResolveTool(name)) return compactJson(record);
  for (const key of ["command", "file_path", "path", "pattern", "query", "url", "description"]) {
    if (typeof record[key] === "string") return record[key];
  }
  return "";
}

/** What an approval shows: a script as code, anything else as JSON. */
export const approvalDetail = (name: string, input: unknown) => (isScript(name, input) ? input.script : formatJson(input));

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

/**
 * What workspaces and series are called, in the app and in the agent's rules. Renaming them is a change here.
 * In code they are collections.
 */
export const COLLECTION_LABEL: Record<CollectionKind, { one: string; many: string; lower: string; lowerMany: string }> = {
  workspace: { one: "Workspace", many: "Workspaces", lower: "workspace", lowerMany: "workspaces" },
  series: { one: "Series", many: "Series", lower: "series", lowerMany: "series" },
};

/** The settings section and the project settings field for them. */
export const COLLECTIONS_TITLE = `${COLLECTION_LABEL.workspace.many} & ${COLLECTION_LABEL.series.lowerMany}`;
export const COLLECTION_PICKER_LABEL = `${COLLECTION_LABEL.workspace.one} / ${COLLECTION_LABEL.series.lower}`;
export const NO_COLLECTION = `No ${COLLECTION_LABEL.workspace.lower}`;
