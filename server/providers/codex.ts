import { Codex, type CodexOptions, type Thread, type ThreadItem, type ThreadOptions } from "@openai/codex-sdk";
import type { ApprovalMode } from "../../shared/protocol.ts";
import type { AppConfig } from "../config.ts";
import { CODEX_DISABLED_RESOLVE_TOOLS } from "../policy.ts";
import { LINKER_MCP_NAME, RESOLVE_MCP_NAME } from "../rules.ts";
import { linkerMcpCommand } from "../agents/commands.ts";
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
  const linker = linkerMcpCommand(config);
  const resolve = config.resolveMcp.command;
  return {
    config: {
      mcp_servers: {
        // Resolve's own server, for its scripting docs and launch_resolve only: Codex has no per-call hook
        // to check a script or back the project up first, so run_script and every tool that writes are off.
        ...(resolve
          ? {
              [RESOLVE_MCP_NAME]: {
                command: slashes(resolve),
                args: [],
                default_tools_approval_mode: "approve",
                disabled_tools: CODEX_DISABLED_RESOLVE_TOOLS,
              },
            }
          : {}),
        // Linker-Edit's tools: the Linker-Edit server asks the user before changes, for Codex too.
        [LINKER_MCP_NAME]: {
          command: slashes(linker.command),
          args: linker.args.map(slashes),
          env: linker.env,
          default_tools_approval_mode: "approve",
          // A first transcription downloads the engine, and an approval waits for the user; Codex gives
          // up on a tool after 60 seconds by default.
          tool_timeout_sec: 3600,
        },
      },
    },
  };
}

/**
 * Codex never asks here; Resolve changes wait for the user in the Linker-Edit server in supervised mode.
 * Like T3 Code, full access runs without a sandbox; supervised keeps writes in the project's folder.
 */
export function codexThreadOptions(config: AppConfig, mode: ApprovalMode = "auto"): ThreadOptions {
  return {
    workingDirectory: config.workspaceDir,
    skipGitRepoCheck: true,
    sandboxMode: mode === "auto" ? "danger-full-access" : "workspace-write",
    approvalPolicy: "never",
  };
}

// No context meter or compaction: the SDK reports tokens summed over a turn, not the context size,
// and runs no slash commands.
export function createCodexSession(config: AppConfig, approvalMode: () => ApprovalMode = () => "auto", resumeId?: string): ProviderSession {
  const codex = new Codex(codexOptions(config));
  // The sandbox is set per thread, so a thread is picked up again with the other one when the mode changes.
  let threadMode = approvalMode();
  let thread: Thread | undefined = resumeId ? codex.resumeThread(resumeId, codexThreadOptions(config, threadMode)) : undefined;
  let turn = 0;

  return {
    resumeId: () => thread?.id ?? null,
    async send(text, sink, signal) {
      const mode = approvalMode();
      if (thread?.id && mode !== threadMode) thread = codex.resumeThread(thread.id, codexThreadOptions(config, mode));
      threadMode = mode;
      thread ??= codex.startThread(codexThreadOptions(config, mode));
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
