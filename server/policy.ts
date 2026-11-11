import type { ApprovalMode } from "../shared/protocol.ts";
import { findResolveTool } from "./resolve/tools.ts";
import { LINKER_MCP_NAME, RESOLVE_MCP_NAME } from "./rules.ts";

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

const LINKER_PREFIX = `mcp__${LINKER_MCP_NAME}__`;
const RESOLVE_PREFIX = `mcp__${RESOLVE_MCP_NAME}__`;

/** Resolve's own MCP tools that only read: its scripting API and docs, and the LUTs and DCTLs it has. */
const RESOLVE_READS = new Set([
  "get_resolve_status",
  "get_whats_new",
  "get_scripting_api",
  "search_scripting_api",
  "get_scripting_docs",
  "list_luts",
  "list_dctls",
]);

/** Resolve's own MCP tools that write files only into Resolve's LUT folder, after the user's OK. */
const RESOLVE_LOOK_WRITES = new Set(["update_dctl", "generate_lut"]);

/** The escape hatch: any Python against Resolve. Always asks, even in "auto" mode, and shows the script. */
export const RUN_SCRIPT = "run_script";

/**
 * Resolve's own MCP tools agents never get: scripts with full system access, deleting LUTs, and starting
 * Resolve. Unknown tools from a newer Resolve land here too, until they're reviewed.
 */
export function isDeniedResolveTool(tool: string): boolean {
  return !RESOLVE_READS.has(tool) && !RESOLVE_LOOK_WRITES.has(tool) && tool !== RUN_SCRIPT;
}

/** Codex has no per-call hook, so it gets only Resolve's read tools; these are switched off for it. */
export const CODEX_DISABLED_RESOLVE_TOOLS = ["run_script", "run_script_unsafe", "update_dctl", "generate_lut", "delete_dctl", "delete_lut", "launch_resolve"];

/** For Claude's tool list: the tools of Resolve's own MCP server it never gets. */
export const CLAUDE_DISALLOWED_RESOLVE_TOOLS = ["run_script_unsafe", "delete_dctl", "delete_lut", "launch_resolve"].map((tool) => `${RESOLVE_PREFIX}${tool}`);

// Reading the edit history: plain or quoted arguments only, so no chaining, substitution or redirects.
const GIT_READ = /^git[ \t]+(log|show|diff|status)([ \t]+("[^"$`\\]*"|'[^']*'|[^\s;&|<>`$()"'\\])+)*$/;

function isGitRead(command: string): boolean {
  // --output writes the result to a file.
  return GIT_READ.test(command.trim()) && !command.includes("--output");
}

/**
 * Does this tool call change the open Resolve project? True for Linker-Edit's write tools and Resolve's
 * run_script, which can do anything. These are the calls the edit history lists after a turn.
 */
export function isResolveWrite(toolName: string): boolean {
  if (toolName.startsWith(LINKER_PREFIX)) return findResolveTool(toolName.slice(LINKER_PREFIX.length))?.kind === "write";
  return toolName === `${RESOLVE_PREFIX}${RUN_SCRIPT}`;
}

/** In "ask" mode: does this tool call need the user's OK before it runs? */
export function needsApproval(toolName: string, input: Record<string, unknown>): boolean {
  if (READ_ONLY_TOOLS.has(toolName)) return false;
  // The Linker-Edit server asks for its own tools' writes itself, for every agent alike.
  if (toolName.startsWith(LINKER_PREFIX)) return false;
  if (toolName.startsWith(RESOLVE_PREFIX)) return !RESOLVE_READS.has(toolName.slice(RESOLVE_PREFIX.length));
  // Claude Code on Windows runs shell commands through PowerShell instead of Bash.
  const shell = toolName === "Bash" || toolName === "PowerShell";
  if (shell && typeof input.command === "string" && isGitRead(input.command)) return false;
  return true;
}

// What scripts may not do through run_script either: each thread belongs to one Resolve project, which
// only the user switches, in Linker-Edit; and media, bins, timelines and projects are never deleted.
const FORBIDDEN_CALLS =
  /\b(LoadProject|LoadCloudProject|CloseProject|CreateProject|CreateCloudProject|DeleteProject|ImportProject|ImportCloudProject|RestoreProject|RestoreCloudProject|ArchiveProject|SetCurrentDatabase|DeleteTimelines|DeleteFolders|DeleteClips|DeleteClipMattes|Quit)\s*\(/;

export const FORBIDDEN_SCRIPT =
  "Linker-Edit doesn't let scripts open, close, create or switch Resolve projects or databases, or delete media, bins or timelines. Use the linker tools (remove_clips takes clips off an AI timeline), or ask the user to switch projects in Linker-Edit's sidebar.";

export type ToolDecision = { allow: true } | { allow: false; message: string };

/**
 * One policy for every agent. Denied tools and forbidden scripts are refused; run_script always waits for
 * the user's OK; other changes wait for it in "ask" mode.
 */
export async function decideToolCall(
  toolName: string,
  input: Record<string, unknown>,
  mode: ApprovalMode,
  ask: () => Promise<boolean>,
): Promise<ToolDecision> {
  if (toolName.startsWith(RESOLVE_PREFIX)) {
    const tool = toolName.slice(RESOLVE_PREFIX.length);
    if (isDeniedResolveTool(tool)) return { allow: false, message: `Linker-Edit doesn't let agents use ${tool}.` };
    if (tool === RUN_SCRIPT) {
      if (typeof input.script === "string" && FORBIDDEN_CALLS.test(input.script)) return { allow: false, message: FORBIDDEN_SCRIPT };
      return (await ask()) ? { allow: true } : { allow: false, message: "The user declined this script." };
    }
  }
  if (mode === "ask" && needsApproval(toolName, input) && !(await ask())) {
    return { allow: false, message: "The user declined this action." };
  }
  return { allow: true };
}
