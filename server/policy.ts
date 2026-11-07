import { RESOLVE_MCP_NAME } from "./rules.ts";

const READ_ONLY_TOOLS = new Set([
  "Read",
  "Glob",
  "Grep",
  "LS",
  "WebSearch",
  "WebFetch",
  "TodoWrite",
  "ToolSearch",
  "ListMcpResourcesTool",
  "ReadMcpResourceTool",
]);

// davinci-resolve-mcp tools are compound: (action, params), and its annotations are per tool, not per
// action. Read actions start with a read verb or end in a report noun ("project_summary").
const READ_ACTION =
  /^(get|list|find|search|describe|inspect|info|status|summary|summarize|count|is|has|probe|check|read|lookup|help|current|query|show|validate|verify|diff|compare|lint|plan|propose)(_|$)|^(?!(commit|export|save|set|create|add|apply|update|clear|delete|remove)_)(.+_)?(capabilities|report|summary|status|inventory|audit|stats|context)$/i;

// A read verb up front doesn't make an action safe: "probe_interchange_roundtrip" imports a timeline
// and "probe_dctl_lifecycle" installs an extension. get_ and list_ are trusted as plain getters.
const WRITE_WORD =
  /(^|_)(lifecycle|roundtrip|install|uninstall|remove|delete|create|apply|import|commit|save|write|rename|replace|insert|duplicate|relink|reset|restore|execute|publish|move|set|add|clear)(_|$)/i;

export function isReadAction(action: string): boolean {
  if (/^(get|list)(_|$)/i.test(action)) return true;
  return READ_ACTION.test(action) && !WRITE_WORD.test(action);
}

const RESOLVE_PREFIX = `mcp__${RESOLVE_MCP_NAME}__`;

/** In "ask" mode: does this tool call need the user's OK before it runs? */
export function needsApproval(toolName: string, input: Record<string, unknown>): boolean {
  if (READ_ONLY_TOOLS.has(toolName)) return false;
  if (toolName.startsWith(RESOLVE_PREFIX)) {
    const tool = toolName.slice(RESOLVE_PREFIX.length);
    if (tool === "knowledge") return false;
    const action = typeof input.action === "string" ? input.action : "";
    return !isReadAction(action);
  }
  return true;
}
