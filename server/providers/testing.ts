import * as path from "node:path";
import type { ChatItem, ContextUsage } from "../../shared/protocol.ts";
import type { AppConfig } from "../config.ts";
import type { ItemPatch, TurnSink } from "./types.ts";

export const FIXTURES = path.join(import.meta.dirname, "fixtures");

export const testConfig: AppConfig = {
  port: 0,
  projectsDir: "C:\\projects",
  workspaceDir: "C:\\work",
  previewDir: "C:\\previews",
  transcriptsDir: "C:\\transcripts",
  toolsDir: "C:\\tools",
  apiToken: "test-token",
  resolveMcp: {
    python: "C:\\mcp\\.venv\\Scripts\\python.exe",
    serverScript: "C:\\mcp\\src\\server.py",
    env: { RESOLVE_SCRIPT_API: "C:\\ProgramData\\Resolve\\Scripting" },
  },
};

/** Collects a turn's chat items the way the app merges them, and answers approvals with `answer`. */
export function collectingSink(answer: (tool: string, input: unknown) => boolean = () => true) {
  const items = new Map<string, ItemPatch>();
  const approvals: { tool: string; input: unknown }[] = [];
  const contexts: (ContextUsage | null)[] = [];
  let notices = 0;
  const upsert = (patch: ItemPatch) => items.set(patch.id, { ...items.get(patch.id), ...patch });
  const sink: TurnSink = {
    upsert,
    notice: (text, tone = "info") => upsert({ id: `notice-${notices++}`, kind: "notice", text, tone }),
    requestApproval: async (tool, input) => {
      approvals.push({ tool, input });
      return answer(tool, input);
    },
    context: (usage) => contexts.push(usage),
  };
  return { sink, approvals, contexts, items: () => [...items.values()] as ChatItem[] };
}
