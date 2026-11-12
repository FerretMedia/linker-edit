import { describe, expect, it } from "vitest";
import { decideToolCall, FORBIDDEN_SCRIPT, isResolveWrite, needsApproval } from "./policy.ts";
import { RESOLVE_TOOLS } from "./resolve/tools.ts";

describe("needsApproval", () => {
  it.each([
    // Agent built-ins
    ["Read", {}, false],
    ["Grep", {}, false],
    ["ToolSearch", {}, false],
    ["Bash", { command: "ls" }, true],
    // Reading the edit history runs unasked; anything that could write still asks.
    ["Bash", { command: "git log -- resolve" }, false],
    ["Bash", { command: 'git show "a1b2c3d:resolve/My Doc/AI_Cut/cuts.json"' }, false],
    ["Bash", { command: "git diff HEAD~1 --stat " }, false],
    ["Bash", { command: "git commit -m x" }, true],
    ["Bash", { command: "git log; rm -rf resolve" }, true],
    ["Bash", { command: "git log\nrm -rf resolve" }, true],
    ["Bash", { command: "git log > out.txt" }, true],
    ["Bash", { command: "git show $(rm x)" }, true],
    ["Bash", { command: "git diff --output=x.patch" }, true],
    ["PowerShell", { command: 'git show "0154f5b:resolve/Demo/AI - Cut/cuts.json"' }, false],
    ["PowerShell", { command: '$p="resolve"; git log -- $p' }, true],
    ["PowerShell", { command: "Remove-Item resolve" }, true],
    ["Write", { file_path: "notes/a.md" }, true],
    ["Edit", {}, true],
    ["Agent", {}, true],
    // Other MCP servers are never trusted as reads.
    ["mcp__gmail__search_threads", {}, true],
    // Linker-Edit's own tools: the server asks for their changes, so the agent's hook never does.
    ["mcp__linker__create_timeline", { name: "Cut" }, false],
    ["mcp__linker__transcribe", { files: [] }, false],
    // Resolve's own server: its docs run unasked, anything else asks.
    ["mcp__resolve__search_scripting_api", { pattern: "Marker" }, false],
    ["mcp__resolve__get_scripting_docs", {}, false],
    ["mcp__resolve__generate_lut", {}, true],
    ["mcp__resolve__run_script", { script: "x = 1" }, true],
    ["mcp__resolve__something_new", {}, true],
  ])("%s %j → %s", (tool, input, expected) => {
    expect(needsApproval(tool, input)).toBe(expected);
  });
});

describe("isResolveWrite", () => {
  it("lists Linker-Edit's write tools and Resolve scripts, not reads, views or transcription", () => {
    const writes = RESOLVE_TOOLS.filter((tool) => isResolveWrite(`mcp__linker__${tool.name}`)).map((tool) => tool.name);
    expect(writes).toEqual(RESOLVE_TOOLS.filter((tool) => tool.kind === "write").map((tool) => tool.name));
    expect(writes).toContain("create_timeline");
    expect(writes).not.toContain("set_current_timeline");
    expect(isResolveWrite("mcp__linker__transcribe")).toBe(false);
    expect(isResolveWrite("mcp__resolve__run_script")).toBe(true);
    expect(isResolveWrite("mcp__resolve__get_scripting_api")).toBe(false);
  });
});

describe("decideToolCall", () => {
  const decide = (tool: string, input: Record<string, unknown>, mode: "ask" | "auto" = "auto", answer = true) => {
    const asked: string[] = [];
    return decideToolCall(tool, input, mode, async () => {
      asked.push(tool);
      return answer;
    }).then((decision) => ({ decision, asked }));
  };

  it("runs a script without asking in auto mode, and asks first in ask mode", async () => {
    const script = { script: "result = project.GetName()" };
    expect(await decide("mcp__resolve__run_script", script, "auto")).toEqual({ decision: { allow: true }, asked: [] });
    expect(await decide("mcp__resolve__run_script", script, "ask")).toEqual({ decision: { allow: true }, asked: ["mcp__resolve__run_script"] });
    expect((await decide("mcp__resolve__run_script", script, "ask", false)).decision).toMatchObject({ allow: false });
  });

  it.each<"ask" | "auto">(["auto", "ask"])("refuses a script that isn't text, in %s mode", async (mode) => {
    expect(await decide("mcp__resolve__run_script", { script: ["x = 1"] }, mode)).toMatchObject({ decision: { allow: false }, asked: [] });
    expect(await decide("mcp__resolve__run_script", {}, mode)).toMatchObject({ decision: { allow: false }, asked: [] });
  });

  it.each([
    'resolve.GetProjectManager().LoadProject("Other")',
    "pm.SetCurrentDatabase({'DbType': 'Disk', 'DbName': 'X'})",
    "project.GetMediaPool().DeleteClips([clip])",
    "pool.DeleteTimelines([t])",
    "pm.CloseProject (project)",
    "project.DeleteAllRenderJobs()",
    "project.DeleteRenderJob(job_id)",
    "project.DeleteRenderPreset('Mine')",
    "item.ReplaceClip('D:/other.mov')",
  ])("refuses a script that switches projects, deletes or swaps media, in both modes: %s", async (script) => {
    for (const mode of ["auto", "ask"] as const) {
      expect(await decide("mcp__resolve__run_script", { script }, mode)).toEqual({ decision: { allow: false, message: FORBIDDEN_SCRIPT }, asked: [] });
    }
  });

  it("lets agents start Resolve, asking only in ask mode", async () => {
    expect(await decide("mcp__resolve__launch_resolve", {}, "auto")).toEqual({ decision: { allow: true }, asked: [] });
    expect(await decide("mcp__resolve__launch_resolve", {}, "ask")).toEqual({ decision: { allow: true }, asked: ["mcp__resolve__launch_resolve"] });
  });

  it.each(["run_script_unsafe", "delete_lut", "delete_dctl", "a_tool_from_a_newer_resolve"])(
    "never lets agents use Resolve's %s, in either mode",
    async (tool) => {
      for (const mode of ["auto", "ask"] as const) {
        const { decision, asked } = await decide(`mcp__resolve__${tool}`, {}, mode);
        expect(decision).toMatchObject({ allow: false });
        expect(asked).toEqual([]);
      }
    },
  );

  it("asks for other changes only in ask mode", async () => {
    expect(await decide("Write", { file_path: "a.md" }, "auto")).toEqual({ decision: { allow: true }, asked: [] });
    expect(await decide("Write", { file_path: "a.md" }, "ask")).toEqual({ decision: { allow: true }, asked: ["Write"] });
    expect(await decide("mcp__resolve__generate_lut", {}, "ask", false)).toMatchObject({ decision: { allow: false } });
  });
});
