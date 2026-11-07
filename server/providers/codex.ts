import { Codex, type CodexOptions, type Thread, type ThreadItem, type ThreadOptions } from "@openai/codex-sdk";
import type { AppConfig } from "../config.ts";
import { RESOLVE_MCP_NAME } from "../rules.ts";
import { truncate, type ProviderSession, type TurnSink } from "./types.ts";

function mcpResultText(item: Extract<ThreadItem, { type: "mcp_tool_call" }>): string | undefined {
  if (item.error) return item.error.message;
  if (!item.result) return undefined;
  return item.result.content.map((part) => (part.type === "text" ? part.text : `[${part.type}]`)).join("\n");
}

function handleItem(item: ThreadItem, sink: TurnSink, turn: number) {
  // Codex item ids restart at item_0 every turn.
  const id = `${turn}:${item.id}`;
  switch (item.type) {
    case "agent_message":
      if (item.text.trim()) sink.upsert({ id, kind: "assistant", text: item.text });
      break;
    case "command_execution":
      sink.upsert({
        id,
        kind: "tool",
        name: "shell",
        input: { command: item.command },
        status: item.status === "in_progress" ? "running" : item.status === "failed" ? "error" : "done",
        output: truncate(item.aggregated_output),
      });
      break;
    case "mcp_tool_call": {
      const output = mcpResultText(item);
      sink.upsert({
        id,
        kind: "tool",
        name: `mcp__${item.server}__${item.tool}`,
        input: item.arguments,
        status: item.status === "in_progress" ? "running" : item.status === "failed" ? "error" : "done",
        ...(output === undefined ? {} : { output: truncate(output) }),
      });
      break;
    }
    case "file_change":
      sink.upsert({
        id,
        kind: "tool",
        name: "edit files",
        input: item.changes,
        status: item.status === "failed" ? "error" : "done",
      });
      break;
    case "web_search":
      sink.upsert({ id, kind: "tool", name: "web search", input: { query: item.query }, status: "done" });
      break;
    case "error":
      sink.notice(item.message, "error");
      break;
  }
}

// Codex's --config parsing hangs on backslash paths; Windows accepts forward slashes.
const slashes = (value: string) => value.replaceAll("\\", "/");

export function codexOptions(config: AppConfig): CodexOptions {
  return {
    config: {
      mcp_servers: {
        [RESOLVE_MCP_NAME]: {
          command: slashes(config.resolveMcp.python),
          args: [slashes(config.resolveMcp.serverScript)],
          env: Object.fromEntries(Object.entries(config.resolveMcp.env).map(([key, value]) => [key, slashes(value)])),
          // The MCP marks its tools destructive; with approvalPolicy "never" Codex would refuse every call.
          default_tools_approval_mode: "approve",
        },
      },
    },
  };
}

// Codex runs non-interactively here: no per-call approvals, writes confined to the workspace.
export function codexThreadOptions(config: AppConfig): ThreadOptions {
  return {
    workingDirectory: config.workspaceDir,
    skipGitRepoCheck: true,
    sandboxMode: "workspace-write",
    approvalPolicy: "never",
  };
}

export function createCodexSession(config: AppConfig): ProviderSession {
  const codex = new Codex(codexOptions(config));
  let thread: Thread | undefined;
  let turn = 0;

  return {
    async send(text, sink, signal) {
      thread ??= codex.startThread(codexThreadOptions(config));
      turn += 1;
      const { events } = await thread.runStreamed(text, { signal });
      for await (const event of events) {
        if (event.type === "item.started" || event.type === "item.updated" || event.type === "item.completed") {
          handleItem(event.item, sink, turn);
        } else if (event.type === "turn.failed") {
          sink.notice(event.error.message, "error");
        } else if (event.type === "error") {
          sink.notice(event.message, "error");
        }
      }
    },
  };
}
