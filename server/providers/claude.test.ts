import type { Options, SDKMessage } from "@anthropic-ai/claude-agent-sdk";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import type { ApprovalMode } from "../../shared/protocol.ts";
import { linkerMcpCommand } from "../agents/commands.ts";
import { CONTEXT_OVERFLOW_NOTICE, createClaudeSession, type RunQuery } from "./claude.ts";
import { readFixture, replayQuery } from "./replay.ts";
import { collectingSink, FIXTURES, testConfig } from "./testing.ts";

const TURN = path.join(FIXTURES, "claude-turn.jsonl");
const signal = () => new AbortController().signal;

const NOW = Date.parse("2026-11-08T12:00:00Z");

function session(runQuery: RunQuery, mode: ApprovalMode = "ask", resumeId?: string) {
  return createClaudeSession(testConfig, () => mode, { runQuery, resumeId, now: () => NOW });
}

/** A stand-in query() that yields the given messages and records each call's prompt and options. */
function scripted(...turns: unknown[][]) {
  const calls: { prompt: string; options: Options }[] = [];
  const runQuery: RunQuery = (params) => {
    calls.push(params);
    const messages = (turns.shift() ?? []) as SDKMessage[];
    return (async function* () {
      yield* messages;
    })();
  };
  return { calls, runQuery };
}

describe("Claude session", () => {
  it("turns a recorded turn into chat items, asking only for changes", async () => {
    const { sink, approvals, items } = collectingSink();
    await session(replayQuery(TURN, 0)).send("check", sink, signal());

    // Linker-Edit's own tools never ask here: the Linker-Edit server asks for their changes itself.
    expect(approvals.map(({ tool }) => tool)).toEqual(["mcp__resolve__run_script", "Write"]);
    const tools = items().filter((item) => item.kind === "tool");
    expect(tools.map((tool) => [tool.name, tool.status])).toEqual([
      ["ToolSearch", "done"],
      ["mcp__resolve__run_script", "done"],
      ["mcp__linker__list_timelines", "done"],
      ["mcp__linker__get_timeline", "done"],
      ["Glob", "done"],
      ["Write", "done"],
    ]);
    expect(tools.every((tool) => typeof tool.output === "string")).toBe(true);
    expect(items().at(-1)).toMatchObject({ kind: "assistant", text: expect.stringContaining("Resolve is reachable") });
    expect(items().some((item) => item.kind === "notice")).toBe(false);
  });

  it("shows a denied call as failed, with the denial", async () => {
    const { sink, items } = collectingSink((tool) => tool !== "Write");
    await session(replayQuery(TURN, 0)).send("check", sink, signal());
    expect(items().find((item) => item.kind === "tool" && item.name === "Write")).toMatchObject({
      status: "error",
      output: "The user declined this action.",
    });
  });

  it("runs a Resolve script without asking in auto mode, but still refuses forbidden ones and Resolve's dangerous tools", async () => {
    const { calls, runQuery } = scripted([]);
    const { sink, approvals } = collectingSink();
    await session(runQuery, "auto").send("go", sink, signal());
    const canUseTool = calls[0]!.options.canUseTool!;
    const ask = (tool: string, input: Record<string, unknown>) =>
      canUseTool(tool, input, { signal: signal(), suggestions: [], toolUseID: "t" } as unknown as Parameters<typeof canUseTool>[2]);
    expect(await ask("mcp__resolve__run_script", { script: "result = project.GetName()" })).toMatchObject({ behavior: "allow" });
    expect(approvals).toEqual([]);
    expect(await ask("mcp__resolve__run_script", { script: 'resolve.GetProjectManager().LoadProject("Other")' })).toMatchObject({ behavior: "deny" });
    expect(await ask("mcp__resolve__run_script_unsafe", { script: "x = 1" })).toMatchObject({ behavior: "deny" });
    expect(await ask("mcp__resolve__search_scripting_api", { pattern: "Marker" })).toMatchObject({ behavior: "allow" });
    expect(approvals).toEqual([]);
    expect(calls[0]!.options.disallowedTools).toContain("mcp__resolve__run_script_unsafe");
    expect(calls[0]!.options.disallowedTools).not.toContain("mcp__resolve__launch_resolve");
  });

  it.each<[ApprovalMode, string, boolean]>([
    ["auto", "bypassPermissions", true],
    ["ask", "default", false],
  ])("in %s mode runs with permissionMode %s, like T3 Code", async (mode, permissionMode, skip) => {
    const { calls, runQuery } = scripted([]);
    await session(runQuery, mode).send("go", collectingSink().sink, signal());
    expect(calls[0]!.options.permissionMode).toBe(permissionMode);
    expect(calls[0]!.options.allowDangerouslySkipPermissions).toBe(skip);
  });

  describe("its PreToolUse hook, which bypassPermissions doesn't skip", () => {
    async function hook(backup?: () => Promise<void>) {
      const { calls, runQuery } = scripted([]);
      const collected = collectingSink();
      const sink = { ...collected.sink, ...(backup ? { backup } : {}) };
      await session(runQuery, "auto").send("go", sink, signal());
      const [matcher] = calls[0]!.options.hooks!.PreToolUse!;
      expect(matcher!.matcher).toBe("mcp__resolve__.*");
      return (tool_name: string, tool_input: Record<string, unknown>) =>
        matcher!.hooks[0]!(
          { hook_event_name: "PreToolUse", tool_name, tool_input, tool_use_id: "t", session_id: "s", transcript_path: "", cwd: "" } as never,
          "t",
          { signal: signal() },
        );
    }
    const denied = (reason: unknown) => ({ hookSpecificOutput: { hookEventName: "PreToolUse", permissionDecision: "deny", permissionDecisionReason: reason } });

    it("refuses forbidden scripts and Resolve's denied tools, without backing up", async () => {
      const backups: string[] = [];
      const run = await hook(async () => void backups.push("backup"));
      expect(await run("mcp__resolve__run_script", { script: "pm.LoadProject('Other')" })).toEqual(denied(expect.stringContaining("doesn't let scripts")));
      expect(await run("mcp__resolve__delete_lut", {})).toEqual(denied("Linker-Edit doesn't let agents use delete_lut."));
      expect(backups).toEqual([]);
    });

    it("backs the project up before a script, and lets the rest through", async () => {
      const backups: string[] = [];
      const run = await hook(async () => void backups.push("backup"));
      expect(await run("mcp__resolve__run_script", { script: "result = 1" })).toEqual({});
      expect(await run("mcp__resolve__launch_resolve", {})).toEqual({});
      expect(await run("mcp__resolve__search_scripting_api", { pattern: "x" })).toEqual({});
      expect(backups).toEqual(["backup"]);
    });

    it("refuses a script when the backup fails", async () => {
      const run = await hook(() => Promise.reject(new Error("Linker-Edit couldn't back up.")));
      expect(await run("mcp__resolve__run_script", { script: "result = 1" })).toEqual(denied("Linker-Edit couldn't back up."));
    });
  });

  it("asks nothing in auto mode", async () => {
    const { sink, approvals, items } = collectingSink();
    await session(replayQuery(TURN, 0), "auto").send("check", sink, signal());
    expect(approvals).toEqual([]);
    expect(items().filter((item) => item.kind === "tool" && item.status === "error")).toEqual([]);
  });

  it("resumes the recorded session on the next turn", async () => {
    const seen: Options[] = [];
    const replay = replayQuery(TURN, 0);
    const claude = session((params) => {
      seen.push(params.options);
      return replay(params);
    });
    await claude.send("one", collectingSink().sink, signal());
    await claude.send("two", collectingSink().sink, signal());
    const [init] = readFixture(TURN);
    expect(seen.map((options) => options.resume)).toEqual([undefined, init!.session_id]);
    expect(claude.resumeId()).toBe(init!.session_id);
    expect(seen[0]!.mcpServers).toEqual({
      resolve: { type: "stdio", command: testConfig.resolveMcp.command, args: [] },
      linker: { type: "stdio", ...linkerMcpCommand(testConfig), alwaysLoad: true },
    });
  });

  it("continues a conversation from before a restart", async () => {
    const { calls, runQuery } = scripted([]);
    const claude = session(runQuery, "ask", "earlier");
    expect(claude.resumeId()).toBe("earlier");
    await claude.send("go on", collectingSink().sink, signal());
    expect(calls[0]!.options.resume).toBe("earlier");
  });

  it("reports how full the context is, with the model's window and when the cache expires", async () => {
    const { sink, contexts } = collectingSink();
    await session(replayQuery(TURN, 0)).send("check", sink, signal());
    expect(contexts.at(-1)).toEqual({
      usedTokens: 44_961,
      maxTokens: 1_000_000,
      cacheExpiresAt: new Date(NOW + 60 * 60_000).toISOString(),
    });
  });

  it("compacts on request, and stops reporting the old size", async () => {
    const boundary = { type: "system", subtype: "compact_boundary", session_id: "s", compact_metadata: { trigger: "manual", pre_tokens: 250_000, post_tokens: 12_000 } };
    const { calls, runQuery } = scripted([{ type: "system", subtype: "init", session_id: "s", mcp_servers: [] }], [boundary]);
    const claude = session(runQuery);
    const { sink, items, contexts } = collectingSink();
    await claude.compact!(sink, signal());
    expect(calls).toHaveLength(0);

    await claude.send("one", sink, signal());
    await claude.compact!(sink, signal());
    expect(calls.map(({ prompt, options }) => [prompt, options.resume])).toEqual([
      ["one", undefined],
      ["/compact", "s"],
    ]);
    expect(items().map((item) => item.kind === "notice" && item.text)).toEqual(["Compacted the chat to save context. It was 250,000 tokens."]);
    expect(contexts).toEqual([null]);
  });

  it("passes the auto-compact window and shortens long Resolve results before Claude reads them", async () => {
    const { calls, runQuery } = scripted([]);
    await createClaudeSession({ ...testConfig, autoCompactWindow: 300_000 }, () => "ask", { runQuery }).send("go", collectingSink().sink, signal());
    const { settings, hooks } = calls[0]!.options;
    expect(settings).toEqual({ autoCompactWindow: 300_000 });
    const [matcher] = hooks!.PostToolUse!;
    expect(matcher!.matcher).toBe("mcp__resolve__.*");
    const hook = (tool_response: unknown) =>
      matcher!.hooks[0]!({ hook_event_name: "PostToolUse", tool_name: "mcp__resolve__run_script", tool_input: {}, tool_response, tool_use_id: "t", session_id: "s", transcript_path: "", cwd: "" }, "t", { signal: signal() });
    expect(await hook([{ type: "text", text: "{}" }])).toEqual({});
    const long = JSON.stringify({ items: Array.from({ length: 2000 }, (_, i) => ({ name: `clip ${i}` })) });
    const output = (await hook([{ type: "text", text: long }])) as { hookSpecificOutput: { updatedToolOutput: { text: string }[] } };
    expect(output.hookSpecificOutput.updatedToolOutput[0]!.text).toContain("1950 more items left out");
  });

  it("leaves Resolve's own MCP server out when this computer has none", async () => {
    const { calls, runQuery } = scripted([]);
    await createClaudeSession({ ...testConfig, resolveMcp: { command: null } }, () => "ask", { runQuery }).send("go", collectingSink().sink, signal());
    expect(Object.keys(calls[0]!.options.mcpServers!)).toEqual(["linker"]);
  });

  it("reports a Resolve MCP that failed to start and a turn that ended badly", async () => {
    const messages = [
      { type: "system", subtype: "init", session_id: "s", mcp_servers: [{ name: "resolve", status: "failed" }, { name: "other", status: "failed" }] },
      { type: "result", subtype: "error_max_turns", session_id: "s" },
      { type: "result", subtype: "success", is_error: true, result: "Prompt is too long", session_id: "s" },
    ] as unknown as SDKMessage[];
    const { sink, items } = collectingSink();
    await session(async function* () {
      yield* messages;
    }).send("check", sink, signal());
    expect(items().map((item) => item.kind === "notice" && item.text)).toEqual([
      'MCP server "resolve" is failed.',
      "Claude stopped (error_max_turns).",
      CONTEXT_OVERFLOW_NOTICE,
    ]);
  });
});
