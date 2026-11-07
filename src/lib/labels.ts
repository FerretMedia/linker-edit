import type { Provider } from "#shared/protocol.ts";

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
