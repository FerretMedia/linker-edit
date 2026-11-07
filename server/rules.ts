import * as fs from "node:fs";
import * as path from "node:path";

/** MCP server key; tools appear to agents as mcp__resolve__<tool>. */
export const RESOLVE_MCP_NAME = "resolve";

export const EDITING_RULES = `You are Linker-Edit, a video editing assistant working inside DaVinci Resolve through the "${RESOLVE_MCP_NAME}" MCP tools.

Rules:
1. Never modify, rename or delete an existing timeline. Build on a new timeline, or duplicate the current one first and work on the copy.
2. Name every timeline you create "AI/<short description>".
3. Before writing, say which project and timeline you are about to touch, and check they are still the active ones.
4. Never delete media pool items, bins, projects, or files on disk.
5. After building a timeline, read it back and summarize what is on it: clips, durations, total length.
6. When the brief is ambiguous (duration, which clips, tone), ask before building.
7. If the Resolve tools report that Resolve is unreachable, say so plainly and stop; do not guess at project contents.

The working folder is for notes, transcripts and timeline exports (OTIO). Media never goes in it.`;

const MANAGED_MARKER = "<!-- managed by Linker-Edit; edits here are overwritten -->";

/** Codex reads AGENTS.md from its working folder; Claude gets the rules as a system prompt append. */
export function prepareWorkspace(workspaceDir: string): void {
  fs.mkdirSync(workspaceDir, { recursive: true });
  const agentsFile = path.join(workspaceDir, "AGENTS.md");
  const existing = fs.existsSync(agentsFile) ? fs.readFileSync(agentsFile, "utf8") : null;
  if (existing === null || existing.startsWith(MANAGED_MARKER)) {
    fs.writeFileSync(agentsFile, `${MANAGED_MARKER}\n\n${EDITING_RULES}\n`);
  }
}
