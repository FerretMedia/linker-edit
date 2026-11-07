import { query } from "@anthropic-ai/claude-agent-sdk";
import * as path from "node:path";
import { createApp } from "./app.ts";
import { APP_ROOT, loadConfig } from "./config.ts";
import { createHistory, readResolveSnapshot } from "./history/history.ts";
import { createServer } from "./http.ts";
import { createClaudeSession } from "./providers/claude.ts";
import { createCodexSession } from "./providers/codex.ts";
import { replayQuery } from "./providers/replay.ts";
import { checkResolve } from "./resolveStatus.ts";
import { prepareWorkspace } from "./rules.ts";

let config: ReturnType<typeof loadConfig>;
try {
  config = loadConfig();
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exit(1);
}
prepareWorkspace(config.workspaceDir);

// Set to a recorded Claude turn (JSONL) to replay it instead of calling Claude: free turns for UI work.
const replay = process.env.LINKER_EDIT_REPLAY;
if (replay) console.log(`Replaying ${replay} instead of calling Claude`);

const app = createApp({
  workspaceDir: config.workspaceDir,
  createSession: (provider, approvalMode) =>
    provider === "codex"
      ? createCodexSession(config)
      : createClaudeSession(config, approvalMode, replay ? replayQuery(replay) : query),
  checkResolve: () => checkResolve(config.resolveMcp),
  // A replayed turn never touched Resolve, so there is nothing to save.
  history: replay
    ? undefined
    : createHistory({ dir: config.workspaceDir, readSnapshot: () => readResolveSnapshot(config.resolveMcp) }),
});

const server = createServer(app, path.join(APP_ROOT, "build"));

server.on("error", (error: NodeJS.ErrnoException) => {
  if (error.code === "EADDRINUSE") {
    console.error(`Port ${config.port} is already in use. Is another Linker-Edit server still running?`);
  } else {
    console.error(error);
  }
  process.exit(1);
});

server.listen(config.port, "127.0.0.1", () => {
  console.log(`Linker-Edit server on http://127.0.0.1:${config.port}`);
  console.log(`Workspace: ${config.workspaceDir}`);
  void app.refreshResolve();
  setInterval(() => {
    if (!app.busy()) void app.refreshResolve();
  }, 15_000);
});
