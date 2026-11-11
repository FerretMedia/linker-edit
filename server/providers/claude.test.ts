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

    expect(approvals.map(({ tool, input }) => [tool, (input as { action?: string }).action])).toEqual([
      ["mcp__resolve__resolve_control", "runtime_mode"],
      ["Write", undefined],
    ]);
    const tools = items().filter((item) => item.kind === "tool");
    expect(tools.map((tool) => [tool.name, tool.status])).toEqual([
      ["ToolSearch", "done"],
      ["mcp__resolve__resolve_control", "done"],
      ["mcp__resolve__project_manager", "done"],
      ["mcp__resolve__timeline", "done"],
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

  it("never lets the agent switch Resolve projects, even in auto mode", async () => {
    const { calls, runQuery } = scripted([]);
    await session(runQuery, "auto").send("go", collectingSink().sink, signal());
    const canUseTool = calls[0]!.options.canUseTool!;
    const ask = (tool: string, action: string) =>
      canUseTool(tool, { action }, { signal: signal(), suggestions: [], toolUseID: "t" } as unknown as Parameters<typeof canUseTool>[2]);
    expect(await ask("mcp__resolve__project_manager", "load")).toMatchObject({ behavior: "deny" });
    expect(await ask("mcp__resolve__project_manager", "get_current")).toMatchObject({ behavior: "allow" });
  });

  it("asks nothing in auto mode", async () => {
    const { sink, approvals } = collectingSink();
    await session(replayQuery(TURN, 0), "auto").send("check", sink, signal());
    expect(approvals).toEqual([]);
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
      resolve: { type: "stdio", command: testConfig.resolveMcp.python, args: [testConfig.resolveMcp.serverScript], env: testConfig.resolveMcp.env },
      linker: { type: "stdio", ...linkerMcpCommand(testConfig) },
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
      matcher!.hooks[0]!({ hook_event_name: "PostToolUse", tool_name: "mcp__resolve__timeline", tool_input: {}, tool_response, tool_use_id: "t", session_id: "s", transcript_path: "", cwd: "" }, "t", { signal: signal() });
    expect(await hook([{ type: "text", text: "{}" }])).toEqual({});
    const long = JSON.stringify({ items: Array.from({ length: 2000 }, (_, i) => ({ name: `clip ${i}` })) });
    const output = (await hook([{ type: "text", text: long }])) as { hookSpecificOutput: { updatedToolOutput: { text: string }[] } };
    expect(output.hookSpecificOutput.updatedToolOutput[0]!.text).toContain("1950 more items left out");
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
