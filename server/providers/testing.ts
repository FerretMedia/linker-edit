import * as path from "node:path";
import type { ChatItem } from "../../shared/protocol.ts";
import type { AppConfig } from "../config.ts";
import type { ItemPatch, TurnSink } from "./types.ts";

export const FIXTURES = path.join(import.meta.dirname, "fixtures");

export const testConfig: AppConfig = {
  port: 0,
  workspaceDir: "C:\\work",
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
  let notices = 0;
  const upsert = (patch: ItemPatch) => items.set(patch.id, { ...items.get(patch.id), ...patch });
  const sink: TurnSink = {
    upsert,
    notice: (text, tone = "info") => upsert({ id: `notice-${notices++}`, kind: "notice", text, tone }),
    requestApproval: async (tool, input) => {
      approvals.push({ tool, input });
      return answer(tool, input);
    },
  };
  return { sink, approvals, items: () => [...items.values()] as ChatItem[] };
}
