import type { CodexOptions, ThreadEvent } from "@openai/codex-sdk";
import * as fs from "node:fs";
import * as path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { createCodexSession } from "./codex.ts";
import { collectingSink, FIXTURES, testConfig } from "./testing.ts";

const fake = vi.hoisted(() => ({ options: [] as unknown[], turns: [] as ThreadEvent[][], resumed: [] as string[] }));

vi.mock("@openai/codex-sdk", () => ({
  Codex: class {
    constructor(options: unknown) {
      fake.options.push(options);
    }
    resumeThread(id: string) {
      fake.resumed.push(id);
      return { ...this.startThread(), id };
    }
    startThread() {
      return {
        runStreamed: async () => ({
          events: (async function* () {
            yield* fake.turns.shift() ?? [];
          })(),
        }),
      };
    }
  },
}));

const recorded = () =>
  fs
    .readFileSync(path.join(FIXTURES, "codex-turn.jsonl"), "utf8")
    .split("\n")
    .filter(Boolean)
    .map((line) => JSON.parse(line) as ThreadEvent);

describe("Codex session", () => {
  it("turns a recorded turn into chat items, keeping each turn's items apart", async () => {
    fake.turns = [recorded(), recorded()];
    const { sink, items } = collectingSink();
    const codex = createCodexSession(testConfig);
    await codex.send("one", sink, new AbortController().signal);
    const firstTurn = items();
    expect(firstTurn.map((item) => [item.kind, item.kind === "tool" ? `${item.name} ${item.status}` : ""])).toEqual([
      ["assistant", ""],
      ["tool", "mcp__resolve__project_settings done"],
      ["assistant", ""],
      ["tool", "edit files done"],
      ["tool", "shell done"],
      ["assistant", ""],
    ]);
    expect(firstTurn[1]).toMatchObject({
      input: { action: "project_summary" },
      output: expect.stringContaining("Untitled Project"),
    });

    await codex.send("two", sink, new AbortController().signal);
    expect(items()).toHaveLength(firstTurn.length * 2);
  });

  it("continues a thread from before a restart", async () => {
    fake.turns = [[]];
    const codex = createCodexSession(testConfig, "thread-1");
    expect(codex.resumeId()).toBe("thread-1");
    await codex.send("go on", collectingSink().sink, new AbortController().signal);
    expect(fake.resumed).toEqual(["thread-1"]);
  });

  it("shows failed turns and stream errors", async () => {
    fake.turns = [
      [
        { type: "turn.failed", error: { message: "usage limit reached" } },
        { type: "error", message: "stream closed" },
      ],
    ];
    const { sink, items } = collectingSink();
    await createCodexSession(testConfig).send("go", sink, new AbortController().signal);
    expect(items().map((item) => item.kind === "notice" && item.text)).toEqual(["usage limit reached", "stream closed"]);
  });

  it("gives Codex forward-slash paths, which its --config parsing needs, and Resolve only behind the guard", () => {
    createCodexSession(testConfig);
    const options = fake.options.at(-1) as CodexOptions;
    expect(JSON.stringify(options.config)).not.toContain("\\\\");
    expect(options.config).toMatchObject({
      mcp_servers: {
        resolve: {
          command: process.execPath.replaceAll("\\", "/"),
          args: [expect.stringMatching(/\/agents\/resolveGuard\.ts$/), "C:/mcp/.venv/Scripts/python.exe", "C:/mcp/src/server.py"],
          env: { LINKER_EDIT_TOKEN: "test-token" },
          disabled_tools: ["project_manager_cloud", "project_manager_database"],
        },
      },
    });
  });
});
