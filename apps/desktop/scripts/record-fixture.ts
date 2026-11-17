// Records one real agent turn as JSONL: raw SDK messages (Claude) or thread events (Codex).
// The fixtures drive the provider tests, and a Claude one can be replayed with LINKER_EDIT_REPLAY.
//
//   node scripts/record-fixture.ts <claude|codex> <out.jsonl> "<prompt>"
//
// Runs on the cheapest models, in a throwaway workspace. Every tool but Bash is allowed, so record
// against a scratch project or with Resolve closed.
import { query } from "@anthropic-ai/claude-agent-sdk";
import { Codex } from "@openai/codex-sdk";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { loadConfig } from "../server/config.ts";
import { createClaudeSession } from "../server/providers/claude.ts";
import { codexOptions, codexThreadOptions } from "../server/providers/codex.ts";
import type { TurnSink } from "../server/providers/types.ts";
import { prepareWorkspace, RESOLVE_MCP_NAME } from "../server/rules.ts";

const [provider, out, prompt] = process.argv.slice(2);
if ((provider !== "claude" && provider !== "codex") || !out || !prompt) {
  console.error('Usage: node scripts/record-fixture.ts <claude|codex> <out.jsonl> "<prompt>"');
  process.exit(1);
}

const KEEP = new Set(["system", "assistant", "user", "tool_progress", "result"]);
const username = os.userInfo().username;

// Fixtures get committed: keep only what the app reads, and drop account and machine details.
function sanitize(message: Record<string, unknown>): Record<string, unknown> | null {
  if (!KEEP.has(String(message.type))) return null;
  if (message.type === "system") {
    if (message.subtype === "compact_boundary") return message;
    if (message.subtype !== "init") return null;
    const { type, subtype, session_id, uuid, model, mcp_servers } = message;
    const resolve = (mcp_servers as { name: string }[]).filter((server) => server.name === RESOLVE_MCP_NAME);
    message = { type, subtype, session_id, uuid, model, mcp_servers: resolve };
  }
  return JSON.parse(JSON.stringify(message).replaceAll(username, "you")) as Record<string, unknown>;
}

const workspaceDir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-record-"));
prepareWorkspace(workspaceDir);
const config = { ...loadConfig(), workspaceDir };
const lines: string[] = [];

if (provider === "claude") {
  const sink: TurnSink = {
    upsert: (patch) => console.log(patch.kind ?? "update", patch.name ?? patch.status ?? patch.text?.slice(0, 80) ?? ""),
    notice: (text) => console.log("notice", text),
    context: (usage) => console.log("context", usage?.usedTokens ?? "unknown"),
    requestApproval: async (tool) => {
      const allow = tool !== "Bash";
      console.log(allow ? "allow" : "deny", tool);
      return allow;
    },
  };
  const session = createClaudeSession(config, () => "ask", {
    runQuery: async function* (params) {
      for await (const message of query({ ...params, options: { ...params.options, model: "haiku" } })) {
        const kept = sanitize(message as unknown as Record<string, unknown>);
        if (kept) lines.push(JSON.stringify(kept));
        yield message;
      }
    },
  });
  await session.send(prompt, sink, new AbortController().signal);
} else {
  const thread = new Codex(codexOptions(config)).startThread({
    ...codexThreadOptions(config),
    model: "gpt-5.6-luna",
    modelReasoningEffort: "low",
  });
  const { events } = await thread.runStreamed(prompt);
  for await (const event of events) {
    lines.push(JSON.stringify(event).replaceAll(username, "you"));
    console.log(event.type, "item" in event ? event.item.type : "");
  }
}

fs.writeFileSync(out, `${lines.join("\n")}\n`);
console.log(`Wrote ${lines.length} messages to ${out}`);
