import { randomUUID } from "node:crypto";
import type { Codex, CodexOptions, Thread, ThreadItem, ThreadOptions } from "@openai/codex-sdk";
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

function handleItem(item: ThreadItem, sink: TurnSink, turn: string) {
  // Codex item ids restart at item_0 every turn, and a session's own count restarts with the server, so a
  // turn gets an id that no earlier turn in the chat has.
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

// Codex always loads the user's global AGENTS.md as well, and has no setting to leave it out.
const VIDEO_ONLY =
  "This is a video edit in Linker-Edit, not a code repository. The AGENTS.md in the working folder is the only one that applies. Ignore the user's global AGENTS.md: do not follow it, and do not read the modes or skills it names.";

// Codex's --config parsing hangs on backslash paths; Windows accepts forward slashes.
const slashes = (value: string) => value.replaceAll("\\", "/");

export function codexOptions(config: AppConfig): CodexOptions {
  const linker = linkerMcpCommand(config);
  const resolve = config.resolveMcp.command;
  return {
    config: {
      developer_instructions: VIDEO_ONLY,
      skills: { include_instructions: false },
      features: { apps: false, plugins: false },
      mcp_servers: {
        // Resolve's own server, for its scripting docs only: Codex has no per-call hook
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
  // The SDK is loaded with the first turn, not when the server starts.
  let codex: Promise<Codex> | null = null;
  // The sandbox is set per thread, so a thread is picked up again with the other one when the mode changes.
  let threadMode = approvalMode();
  let thread: Thread | undefined;

  return {
    resumeId: () => thread?.id ?? resumeId ?? null,
    async send(text, sink, signal) {
      const sdk = await (codex ??= import("@openai/codex-sdk").then(({ Codex }) => new Codex(codexOptions(config))));
      const mode = approvalMode();
      if (!thread && resumeId) thread = sdk.resumeThread(resumeId, codexThreadOptions(config, mode));
      else if (thread?.id && mode !== threadMode) thread = sdk.resumeThread(thread.id, codexThreadOptions(config, mode));
      threadMode = mode;
      thread ??= sdk.startThread(codexThreadOptions(config, mode));
      const turn = randomUUID();
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
