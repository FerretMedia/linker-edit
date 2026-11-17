import type { CodexOptions, ThreadEvent } from "@openai/codex-sdk";
import * as fs from "node:fs";
import * as path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { createCodexSession } from "./codex.ts";
import { collectingSink, FIXTURES, testConfig } from "./testing.ts";

const fake = vi.hoisted(() => ({ options: [] as unknown[], turns: [] as ThreadEvent[][], resumed: [] as string[], sandboxes: [] as string[] }));

vi.mock("@openai/codex-sdk", () => ({
  Codex: class {
    constructor(options: unknown) {
      fake.options.push(options);
    }
    resumeThread(id: string, options?: { sandboxMode: string }) {
      fake.resumed.push(id);
      return { ...this.startThread(options), id };
    }
    startThread(options?: { sandboxMode: string }) {
      if (options) fake.sandboxes.push(options.sandboxMode);
      return {
        id: "thread-new",
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

  it("never reuses an earlier turn's item ids in a session started after a restart", async () => {
    fake.turns = [recorded(), recorded()];
    const { sink, items } = collectingSink();
    await createCodexSession(testConfig).send("one", sink, new AbortController().signal);
    const before = items().length;
    await createCodexSession(testConfig, () => "auto", "thread-new").send("two", sink, new AbortController().signal);
    expect(items()).toHaveLength(before * 2);
    fake.resumed = [];
  });

  it("continues a thread from before a restart", async () => {
    fake.turns = [[]];
    const codex = createCodexSession(testConfig, () => "auto", "thread-1");
    expect(codex.resumeId()).toBe("thread-1");
    await codex.send("go on", collectingSink().sink, new AbortController().signal);
    expect(fake.resumed).toEqual(["thread-1"]);
  });

  it("runs without a sandbox with full access and sandboxed when supervised, like T3 Code", async () => {
    fake.turns = [[], [], []];
    fake.sandboxes = [];
    fake.resumed = [];
    let mode: "ask" | "auto" = "auto";
    const codex = createCodexSession(testConfig, () => mode);
    await codex.send("one", collectingSink().sink, new AbortController().signal);
    mode = "ask";
    await codex.send("two", collectingSink().sink, new AbortController().signal);
    await codex.send("three", collectingSink().sink, new AbortController().signal);
    expect(fake.sandboxes).toEqual(["danger-full-access", "workspace-write"]);
    expect(fake.resumed).toEqual(["thread-new"]);
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

  it("gives Codex forward-slash paths, which its --config parsing needs, and only Resolve's read tools", () => {
    createCodexSession(testConfig);
    const options = fake.options.at(-1) as CodexOptions;
    expect(JSON.stringify(options.config)).not.toContain("\\\\");
    expect(JSON.stringify(options.config)).not.toContain("launch_resolve");
    expect(options.config).toMatchObject({
      mcp_servers: {
        resolve: {
          command: "C:/Program Files/Blackmagic Design/DaVinci Resolve/ResolveMCP.exe",
          disabled_tools: expect.arrayContaining(["run_script", "run_script_unsafe", "delete_lut"]),
        },
        linker: { env: { LINKER_EDIT_TOKEN: "test-token" } },
      },
    });
  });

  it("leaves Resolve's own MCP server out when this computer has none", () => {
    createCodexSession({ ...testConfig, resolveMcp: { command: null } });
    const options = fake.options.at(-1) as CodexOptions;
    expect(Object.keys((options.config as { mcp_servers: object }).mcp_servers)).toEqual(["linker"]);
  });
});
