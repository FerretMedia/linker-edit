import type { Options, SDKMessage } from "@anthropic-ai/claude-agent-sdk";
import * as fs from "node:fs";
import type { RunQuery } from "./claude.ts";

// Slow enough to watch a turn stream in the UI, fast enough for the e2e test.
const STEP_MS = 150;

export function readFixture(file: string): SDKMessage[] {
  return fs
    .readFileSync(file, "utf8")
    .split("\n")
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line) as SDKMessage);
}

/**
 * Stands in for the Agent SDK's query(): replays a recorded Claude turn (one SDK message per JSONL
 * line) and asks canUseTool for each tool call, like the real SDK. A denied call replays as an error.
 */
export function replayQuery(fixtureFile: string, stepMs = STEP_MS): RunQuery {
  let run = 0;
  return ({ options }) => {
    run += 1;
    return replay(readFixture(fixtureFile), options, stepMs, run);
  };
}

async function* replay(messages: SDKMessage[], options: Options, stepMs: number, run: number) {
  const signal = options.abortController?.signal ?? new AbortController().signal;
  const denied = new Map<string, string>();
  // Recorded ids repeat on every replay, and the app merges chat items by id.
  const rid = (id: string) => `${id}~${run}`;

  for (const message of messages) {
    await abortable(new Promise((resolve) => setTimeout(resolve, stepMs)), signal);

    if (message.type === "assistant") {
      const content = message.message.content.map((block) =>
        block.type === "tool_use" ? { ...block, id: rid(block.id) } : block,
      );
      yield { ...message, uuid: rid(message.uuid), message: { ...message.message, content } } as SDKMessage;
      for (const block of content) {
        if (block.type !== "tool_use" || !options.canUseTool) continue;
        const input = block.input as Record<string, unknown>;
        const decision = await abortable(options.canUseTool(block.name, input, { signal, toolUseID: block.id, requestId: block.id }), signal);
        if (decision?.behavior === "deny") denied.set(block.id, decision.message);
      }
    } else if (message.type === "user" && Array.isArray(message.message.content)) {
      const content = message.message.content.map((block) => {
        if (block.type !== "tool_result") return block;
        const id = rid(block.tool_use_id);
        const denial = denied.get(id);
        return denial === undefined
          ? { ...block, tool_use_id: id }
          : { ...block, tool_use_id: id, is_error: true, content: denial };
      });
      yield { ...message, message: { ...message.message, content } } as SDKMessage;
    } else {
      yield message;
    }
  }
}

function abortable<T>(promise: Promise<T>, signal: AbortSignal): Promise<T> {
  return new Promise((resolve, reject) => {
    const onAbort = () => reject(new Error("Replay aborted"));
    if (signal.aborted) return onAbort();
    signal.addEventListener("abort", onAbort, { once: true });
    promise.then(
      (value) => {
        signal.removeEventListener("abort", onAbort);
        resolve(value);
      },
      reject,
    );
  });
}
