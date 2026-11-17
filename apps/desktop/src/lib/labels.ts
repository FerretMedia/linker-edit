import type { ClipChange, CollectionKind, Provider, ResolveLink } from "#shared/protocol.ts";

// Resolve's own MCP server, and Linker-Edit's, whose tools other than transcribe all work in Resolve.
const RESOLVE_PREFIX = "mcp__resolve__";
const LINKER_PREFIX = "mcp__linker__";

export const PROVIDER_LABEL: Record<Provider, string> = { claude: "Claude", codex: "Codex" };

export const isResolveTool = (name: string) =>
  name.startsWith(RESOLVE_PREFIX) || (name.startsWith(LINKER_PREFIX) && name !== `${LINKER_PREFIX}transcribe`);

const isScript = (name: string, input: unknown): input is { script: string } =>
  name === `${RESOLVE_PREFIX}run_script` && typeof (input as { script?: unknown } | null)?.script === "string";

/** "mcp__linker__create_timeline" → "create_timeline"; other tools keep their name. */
export function toolLabel(name: string, _input: unknown): string {
  return isResolveTool(name) ? name.slice(name.indexOf("__", 5) + 2) : name;
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
