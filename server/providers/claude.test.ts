import type { Options, SDKMessage } from "@anthropic-ai/claude-agent-sdk";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import type { ApprovalMode } from "../../shared/protocol.ts";
import { createClaudeSession, type RunQuery } from "./claude.ts";
import { readFixture, replayQuery } from "./replay.ts";
import { collectingSink, FIXTURES, testConfig } from "./testing.ts";

const TURN = path.join(FIXTURES, "claude-turn.jsonl");
const signal = () => new AbortController().signal;

function session(runQuery: RunQuery, mode: ApprovalMode = "ask") {
  return createClaudeSession(testConfig, () => mode, runQuery);
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
    expect(seen[0]!.mcpServers).toEqual({
      resolve: { type: "stdio", command: testConfig.resolveMcp.python, args: [testConfig.resolveMcp.serverScript], env: testConfig.resolveMcp.env },
    });
  });

  it("reports a Resolve MCP that failed to start and a turn that ended badly", async () => {
    const messages = [
      { type: "system", subtype: "init", session_id: "s", mcp_servers: [{ name: "resolve", status: "failed" }, { name: "other", status: "failed" }] },
      { type: "result", subtype: "error_max_turns", session_id: "s" },
    ] as unknown as SDKMessage[];
    const { sink, items } = collectingSink();
    await session(async function* () {
      yield* messages;
    }).send("check", sink, signal());
    expect(items().map((item) => item.kind === "notice" && item.text)).toEqual([
      'MCP server "resolve" is failed.',
      "Claude stopped (error_max_turns).",
    ]);
  });
});
