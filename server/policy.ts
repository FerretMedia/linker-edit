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

/**
 * The escape hatch: any Python against Resolve. Scripts that break the rules are refused in every mode;
 * the rest run like any other change, so they ask, showing the script, only in "ask" mode.
 */
export const RUN_SCRIPT = "run_script";

/** Starts Resolve when it isn't running. It changes no project, so it's fine for agents. */
export const LAUNCH_RESOLVE = "launch_resolve";

/**
 * Resolve's own MCP tools agents never get: scripts with full system access and deleting LUTs. Unknown
 * tools from a newer Resolve land here too, until they're reviewed.
 */
export function isDeniedResolveTool(tool: string): boolean {
  return !RESOLVE_READS.has(tool) && !RESOLVE_LOOK_WRITES.has(tool) && tool !== RUN_SCRIPT && tool !== LAUNCH_RESOLVE;
}

/**
 * Codex has no per-call hook to check a script or back the project up first, so it gets only Resolve's
 * read tools and launch_resolve; these are switched off for it.
 */
export const CODEX_DISABLED_RESOLVE_TOOLS = ["run_script", "run_script_unsafe", "update_dctl", "generate_lut", "delete_dctl", "delete_lut"];

/** For Claude's tool list: the tools of Resolve's own MCP server it never gets. */
export const CLAUDE_DISALLOWED_RESOLVE_TOOLS = ["run_script_unsafe", "delete_dctl", "delete_lut"].map((tool) => `${RESOLVE_PREFIX}${tool}`);

/** Resolve's own run_script, as agents see it. It changes the project, so the project is backed up first. */
export const RESOLVE_RUN_SCRIPT = `${RESOLVE_PREFIX}${RUN_SCRIPT}`;

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
  return toolName === RESOLVE_RUN_SCRIPT;
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

// What scripts may not do through run_script, in any mode: each thread belongs to one Resolve project,
// which only the user switches, in Linker-Edit; media, bins, timelines, projects, the user's render queue
// and render presets are never deleted; and a clip's media is never swapped for another file. With full
// access no one reads a script before it runs, so this check is what stops these mistakes.
const FORBIDDEN_CALLS =
  /\b(LoadProject|LoadCloudProject|CloseProject|CreateProject|CreateCloudProject|DeleteProject|ImportProject|ImportCloudProject|RestoreProject|RestoreCloudProject|ArchiveProject|SetCurrentDatabase|DeleteTimelines|DeleteFolders|DeleteClips|DeleteClipMattes|DeleteRenderJob|DeleteAllRenderJobs|DeleteRenderPreset|ReplaceClip|Quit)\s*\(/;

export const FORBIDDEN_SCRIPT =
  "Linker-Edit doesn't let scripts open, close, create or switch Resolve projects or databases, delete media, bins, timelines, render jobs or render presets, or replace a clip's media. Use the linker tools (remove_clips takes clips off an AI timeline), or ask the user to switch projects in Linker-Edit's sidebar.";

export type ToolDecision = { allow: true } | { allow: false; message: string };

/**
 * What Linker-Edit refuses whatever the mode: Resolve's denied tools, and scripts that can't be checked or
 * break the rules. Null when the call may go on. Claude runs this in a PreToolUse hook too, because full
 * access (bypassPermissions) never calls canUseTool.
 */
export function refusal(toolName: string, input: Record<string, unknown>): string | null {
  if (!toolName.startsWith(RESOLVE_PREFIX)) return null;
  const tool = toolName.slice(RESOLVE_PREFIX.length);
  if (isDeniedResolveTool(tool)) return `Linker-Edit doesn't let agents use ${tool}.`;
  if (tool === RUN_SCRIPT) {
    // A script that can't be checked doesn't run.
    if (typeof input.script !== "string") return "run_script needs the script as text.";
    if (FORBIDDEN_CALLS.test(input.script)) return FORBIDDEN_SCRIPT;
  }
  return null;
}

/**
 * One policy for every agent, like T3 Code's runtime modes: "auto" is full access, "ask" is supervised.
 * Denied tools and forbidden scripts are refused in both; in "ask" mode, changes wait for the user's OK.
 */
export async function decideToolCall(
  toolName: string,
  input: Record<string, unknown>,
  mode: ApprovalMode,
  ask: () => Promise<boolean>,
): Promise<ToolDecision> {
  const refused = refusal(toolName, input);
  if (refused) return { allow: false, message: refused };
  if (mode === "ask" && needsApproval(toolName, input) && !(await ask())) {
    return { allow: false, message: "The user declined this action." };
  }
  return { allow: true };
}
