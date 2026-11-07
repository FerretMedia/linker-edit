import { query, type Options, type SDKMessage } from "@anthropic-ai/claude-agent-sdk";
import type { ApprovalMode } from "../../shared/protocol.ts";
import type { AppConfig } from "../config.ts";
import { needsApproval } from "../policy.ts";
import { EDITING_RULES, RESOLVE_MCP_NAME } from "../rules.ts";
import { truncate, type ProviderSession, type TurnSink } from "./types.ts";

function toolResultText(content: unknown): string {
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {
    return content
      .map((part: { type?: string; text?: string }) => (part.type === "text" ? part.text : `[${part.type}]`))
      .join("\n");
  }
  return JSON.stringify(content);
}

/** The slice of the Agent SDK's query() a session uses; replays and tests stand in for it. */
export type RunQuery = (params: { prompt: string; options: Options }) => AsyncIterable<SDKMessage>;

export function createClaudeSession(
  config: AppConfig,
  approvalMode: () => ApprovalMode,
  runQuery: RunQuery = query,
): ProviderSession {
  let sessionId: string | undefined;

  function handle(message: SDKMessage, sink: TurnSink) {
    if ("session_id" in message && typeof message.session_id === "string") sessionId = message.session_id;

    switch (message.type) {
      case "system":
        if (message.subtype === "init") {
          for (const server of message.mcp_servers) {
            if (server.name === RESOLVE_MCP_NAME && server.status !== "connected") {
              sink.notice(`MCP server "${server.name}" is ${server.status}.`, "error");
            }
          }
        }
        break;
      case "assistant":
        message.message.content.forEach((block, index) => {
          if (block.type === "text" && block.text.trim()) {
            sink.upsert({ id: `${message.uuid}:${index}`, kind: "assistant", text: block.text });
          } else if (block.type === "tool_use") {
            sink.upsert({ id: block.id, kind: "tool", name: block.name, input: block.input, status: "running" });
          }
        });
        break;
      case "user":
        if (Array.isArray(message.message.content)) {
          for (const block of message.message.content) {
            if (block.type === "tool_result") {
              sink.upsert({
                id: block.tool_use_id,
                status: block.is_error ? "error" : "done",
                output: truncate(toolResultText(block.content)),
              });
            }
          }
        }
        break;
      case "result":
        if (message.subtype !== "success") {
          sink.notice(`Claude stopped (${message.subtype}).`, "error");
        }
        break;
    }
  }

  return {
    async send(text, sink, signal) {
      const abortController = new AbortController();
      signal.addEventListener("abort", () => abortController.abort(), { once: true });

      const run = runQuery({
        prompt: text,
        options: {
          cwd: config.workspaceDir,
          resume: sessionId,
          abortController,
          systemPrompt: { type: "preset", preset: "claude_code", append: EDITING_RULES },
          mcpServers: {
            [RESOLVE_MCP_NAME]: {
              type: "stdio",
              command: config.resolveMcp.python,
              args: [config.resolveMcp.serverScript],
              env: config.resolveMcp.env,
            },
          },
          permissionMode: "default",
          canUseTool: async (toolName, input) => {
            if (approvalMode() === "ask" && needsApproval(toolName, input)) {
              const allowed = await sink.requestApproval(toolName, input);
              if (!allowed) return { behavior: "deny", message: "The user declined this action." };
            }
            return { behavior: "allow", updatedInput: input };
          },
        },
      });

      for await (const message of run) handle(message, sink);
    },
  };
}
