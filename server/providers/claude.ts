import { query, type Options, type SDKMessage } from "@anthropic-ai/claude-agent-sdk";
import type { ApprovalMode } from "../../shared/protocol.ts";
import type { AppConfig } from "../config.ts";
import { CLAUDE_DISALLOWED_RESOLVE_TOOLS, decideToolCall } from "../policy.ts";
import { EDITING_RULES, LINKER_MCP_NAME, RESOLVE_MCP_NAME } from "../rules.ts";
import { linkerMcpCommand } from "../agents/commands.ts";
import { shortenToolOutput } from "./shorten.ts";
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

export interface ClaudeSessionOptions {
  runQuery?: RunQuery;
  /** A session id from an earlier run, to continue that conversation. */
  resumeId?: string;
  now?: () => number;
}

export const CONTEXT_OVERFLOW_NOTICE =
  "The chat no longer fits in Claude's context. Compact it from the context meter, or start a new chat.";

type MessageUsage = {
  input_tokens: number;
  output_tokens: number;
  cache_creation_input_tokens?: number | null;
  cache_read_input_tokens?: number | null;
  cache_creation?: { ephemeral_1h_input_tokens?: number; ephemeral_5m_input_tokens?: number } | null;
};

/** How long the cache this request wrote lives; undefined when it wrote none, as when fully served from cache. */
function cacheTtlMs(usage: MessageUsage): number | undefined {
  if ((usage.cache_creation?.ephemeral_1h_input_tokens ?? 0) > 0) return 60 * 60_000;
  if ((usage.cache_creation?.ephemeral_5m_input_tokens ?? 0) > 0) return 5 * 60_000;
  return undefined;
}

export function createClaudeSession(
  config: AppConfig,
  approvalMode: () => ApprovalMode,
  { runQuery = query, resumeId, now = Date.now }: ClaudeSessionOptions = {},
): ProviderSession {
  let sessionId = resumeId;
  let maxTokens: number | null = null;
  let usedTokens: number | null = null;
  let model: string | undefined;
  // A read served fully from cache still refreshes it for the TTL it was written with.
  let ttlMs: number | undefined;

  function reportContext(sink: TurnSink, cacheExpiresAt: string | null) {
    if (usedTokens !== null) sink.context({ usedTokens, maxTokens, cacheExpiresAt });
  }

  function handle(message: SDKMessage, sink: TurnSink) {
    if ("session_id" in message && typeof message.session_id === "string") sessionId = message.session_id;

    switch (message.type) {
      case "system":
        if (message.subtype === "init") {
          for (const server of message.mcp_servers) {
            if ((server.name === RESOLVE_MCP_NAME || server.name === LINKER_MCP_NAME) && server.status !== "connected") {
              sink.notice(`MCP server "${server.name}" is ${server.status}.`, "error");
            }
          }
        } else if (message.subtype === "compact_boundary") {
          sink.notice(`Compacted the chat to save context. It was ${message.compact_metadata.pre_tokens.toLocaleString("en-US")} tokens.`);
          // post_tokens leaves out the system prompt and tools, so the size is unknown until the next request.
          usedTokens = null;
          ttlMs = undefined;
          sink.context(null);
        }
        break;
      case "assistant":
        // Subagents have their own context; only the main conversation's fills this one.
        if (message.parent_tool_use_id === null) {
          model = message.message.model;
          const usage = message.message.usage as MessageUsage;
          usedTokens =
            usage.input_tokens +
            (usage.cache_creation_input_tokens ?? 0) +
            (usage.cache_read_input_tokens ?? 0) +
            usage.output_tokens;
          ttlMs = cacheTtlMs(usage) ?? ttlMs;
          reportContext(sink, ttlMs === undefined ? null : new Date(now() + ttlMs).toISOString());
        }
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
      case "result": {
        // modelUsage also lists models that only ran subagents or the compaction itself.
        const window = model === undefined ? undefined : message.modelUsage?.[model]?.contextWindow;
        if (window) {
          maxTokens = window;
          reportContext(sink, ttlMs === undefined ? null : new Date(now() + ttlMs).toISOString());
        }
        if (message.subtype !== "success") {
          sink.notice(`Claude stopped (${message.subtype}).`, "error");
        } else if (message.is_error && /prompt is too long|context window/i.test(message.result)) {
          sink.notice(CONTEXT_OVERFLOW_NOTICE, "error");
        }
        break;
      }
    }
  }

  async function send(text: string, sink: TurnSink, signal: AbortSignal) {
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
          // Resolve's own server, for its scripting docs and, after the user's OK, run_script.
          ...(config.resolveMcp.command ? { [RESOLVE_MCP_NAME]: { type: "stdio" as const, command: config.resolveMcp.command, args: [] } } : {}),
          [LINKER_MCP_NAME]: { type: "stdio", ...linkerMcpCommand(config) },
        },
        disallowedTools: CLAUDE_DISALLOWED_RESOLVE_TOOLS,
        ...(config.autoCompactWindow === undefined ? {} : { settings: { autoCompactWindow: config.autoCompactWindow } }),
        hooks: {
          PostToolUse: [
            {
              matcher: `mcp__${RESOLVE_MCP_NAME}__.*`,
              hooks: [
                async (input) => {
                  if (input.hook_event_name !== "PostToolUse") return {};
                  const shortened = shortenToolOutput(input.tool_response);
                  return shortened === undefined
                    ? {}
                    : { hookSpecificOutput: { hookEventName: "PostToolUse", updatedToolOutput: shortened } };
                },
              ],
            },
          ],
        },
        permissionMode: "default",
        canUseTool: async (toolName, input) => {
          const decision = await decideToolCall(toolName, input, approvalMode(), () => sink.requestApproval(toolName, input));
          return decision.allow ? { behavior: "allow", updatedInput: input } : { behavior: "deny", message: decision.message };
        },
      },
    });

    for await (const message of run) handle(message, sink);
  }

  return {
    send,
    // The SDK runs slash commands sent as the prompt; there is nothing to compact before the first turn.
    compact: async (sink, signal) => {
      if (sessionId) await send("/compact", sink, signal);
    },
    resumeId: () => sessionId ?? null,
  };
}
