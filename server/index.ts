import { query } from "@anthropic-ai/claude-agent-sdk";
import * as path from "node:path";
import { createApp } from "./app.ts";
import { APP_ROOT, loadConfigOrExit } from "./config.ts";
import { createHistory, readResolveSnapshot } from "./history/history.ts";
import { createServer } from "./http.ts";
import { createPreviewRenderer } from "./preview/preview.ts";
import { createClaudeSession } from "./providers/claude.ts";
import { createCodexSession } from "./providers/codex.ts";
import { replayQuery } from "./providers/replay.ts";
import { checkResolve } from "./resolveStatus.ts";
import { prepareWorkspace } from "./rules.ts";
import { isNewer, latestVersion, VERSION } from "./updates.ts";

const config = loadConfigOrExit();
prepareWorkspace(config.workspaceDir);

// Set to a recorded Claude turn (JSONL) to replay it instead of calling Claude: free turns for UI work.
const replay = process.env.LINKER_EDIT_REPLAY;
if (replay) console.log(`Replaying ${replay} instead of calling Claude`);

const app = createApp({
  version: VERSION,
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
  preview: createPreviewRenderer({ dir: config.previewDir, mcp: config.resolveMcp }),
});

export const url = `http://127.0.0.1:${config.port}`;
export const server = createServer(app, path.join(APP_ROOT, "build"));

server.on("error", (error: NodeJS.ErrnoException) => {
  if (error.code === "EADDRINUSE") {
    console.error(`Port ${config.port} is already in use. Is another Linker-Edit server still running?`);
  } else {
    console.error(error);
  }
  process.exit(1);
});

server.listen(config.port, "127.0.0.1", () => {
  console.log(`Linker-Edit ${VERSION} on ${url}`);
  console.log(`Workspace: ${config.workspaceDir}`);
  void app.refreshResolve();
  setInterval(() => {
    if (!app.busy()) void app.refreshResolve();
  }, 15_000);
});

// A checkout runs as 0.0.0 and has nothing to update to.
if (VERSION !== "0.0.0") {
  const checkForUpdate = () =>
    void latestVersion().then((latest) => app.setUpdate(latest && isNewer(latest, VERSION) ? latest : null));
  checkForUpdate();
  setInterval(checkForUpdate, 24 * 60 * 60 * 1000);
}
