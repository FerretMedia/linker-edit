import * as fs from "node:fs";
import { describe, expect, it } from "vitest";
import { loadConfig } from "./config.ts";
import { isReadAction, needsApproval } from "./policy.ts";

describe("needsApproval", () => {
  it.each([
    // Agent built-ins
    ["Read", {}, false],
    ["Grep", {}, false],
    ["ToolSearch", {}, false],
    ["Bash", { command: "ls" }, true],
    ["Write", { file_path: "notes/a.md" }, true],
    ["Edit", {}, true],
    ["Agent", {}, true],
    // Other MCP servers are never trusted as reads.
    ["mcp__gmail__search_threads", { action: "list" }, true],
    // Resolve: the action decides.
    ["mcp__resolve__knowledge", { action: "anything" }, false],
    ["mcp__resolve__timeline", { action: "list" }, false],
    ["mcp__resolve__project_manager", { action: "get_current" }, false],
    ["mcp__resolve__project_settings", { action: "project_summary" }, false],
    ["mcp__resolve__edit_engine", { action: "plan_silence_ripple" }, false],
    ["mcp__resolve__timeline", { action: "create_empty" }, true],
    ["mcp__resolve__timeline", { action: "set_current" }, true],
    ["mcp__resolve__timeline", { action: "delete" }, true],
    ["mcp__resolve__timeline", { action: "export_report" }, true],
    ["mcp__resolve__timeline", {}, true],
    ["mcp__resolve__timeline", { action: 42 }, true],
    // A read verb up front, but the action writes.
    ["mcp__resolve__timeline", { action: "probe_interchange_roundtrip" }, true],
    ["mcp__resolve__script_plugin", { action: "probe_dctl_lifecycle" }, true],
    ["mcp__resolve__script_plugin", { action: "check_and_install" }, true],
    ["mcp__resolve__timeline", { action: "rename_timeline_report" }, true],
  ])("%s %j → %s", (tool, input, expected) => {
    expect(needsApproval(tool, input)).toBe(expected);
  });
});

// Every action davinci-resolve-mcp dispatches on, so an MCP upgrade shows which new actions skip approval.
function resolveActions(serverScript: string): string[] {
  const source = fs.readFileSync(serverScript, "utf8");
  const names = new Set<string>();
  for (const match of source.matchAll(/action\s*==\s*"([a-z0-9_]+)"/g)) names.add(match[1]!);
  for (const match of source.matchAll(/action\s+in\s*[({[]([^)}\]]*)[)}\]]/g)) {
    for (const name of match[1]!.matchAll(/"([a-z0-9_]+)"/g)) names.add(name[1]!);
  }
  return [...names].sort();
}

const serverScript = loadConfig().resolveMcp.serverScript;

describe.skipIf(!fs.existsSync(serverScript))("davinci-resolve-mcp actions that run without approval", () => {
  it("match the reviewed list", async () => {
    const actions = resolveActions(serverScript);
    expect(actions.length).toBeGreaterThan(100);
    // Review the diff before accepting it with `npx vitest -u`: every line here runs unasked.
    await expect(`${actions.filter(isReadAction).join("\n")}\n`).toMatchFileSnapshot(
      "__snapshots__/resolve-read-actions.txt",
    );
  });
});
