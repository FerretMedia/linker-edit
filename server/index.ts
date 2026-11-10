import { query } from "@anthropic-ai/claude-agent-sdk";
import * as path from "node:path";
import { createApp } from "./app.ts";
import { APP_ROOT, loadConfigOrExit } from "./config.ts";
import { createHistory, readResolveSnapshot } from "./history/history.ts";
import { createServer } from "./http.ts";
import { createPreviewRenderer } from "./preview/preview.ts";
import { fileProjectStore } from "./projects/store.ts";
import { createClaudeSession } from "./providers/claude.ts";
import { createCodexSession } from "./providers/codex.ts";
import { replayQuery } from "./providers/replay.ts";
import { createRemoteAccess } from "./remote.ts";
import { createResolveProjects } from "./resolveProjects.ts";
import { checkResolve } from "./resolveStatus.ts";
import { prepareWorkspace } from "./rules.ts";
import { prepareTools } from "./transcribe/tools.ts";
import { createTranscriber } from "./transcribe/transcriber.ts";
import { isNewer, latestVersion, VERSION } from "./updates.ts";

const config = loadConfigOrExit();

// Set to a recorded Claude turn (JSONL) to replay it instead of calling Claude: free turns for UI work.
const replay = process.env.LINKER_EDIT_REPLAY;
if (replay) console.log(`Replaying ${replay} instead of calling Claude`);

const remote = createRemoteAccess({ port: config.port, servePort: config.tailscalePort });

const app = createApp({
  version: VERSION,
  store: fileProjectStore({ root: config.projectsDir, legacyDir: config.workspaceDir }),
  createSession: (provider, approvalMode, dir, resumeId) => {
    const projectConfig = { ...config, workspaceDir: dir };
    return provider === "codex"
      ? createCodexSession(projectConfig, resumeId)
      : createClaudeSession(projectConfig, approvalMode, { runQuery: replay ? replayQuery(replay) : query, resumeId });
  },
  checkResolve: () => checkResolve(config.resolveMcp),
  // A replayed turn never touched Resolve, so there is nothing to save.
  createHistory: replay
    ? undefined
    : (dir, checkProject) => createHistory({ dir, readSnapshot: () => readResolveSnapshot(config.resolveMcp), checkProject }),
  preview: createPreviewRenderer({ dir: config.previewDir, mcp: config.resolveMcp }),
  resolveProjects: createResolveProjects(config.resolveMcp),
  prepareFolder: prepareWorkspace,
  transcriber: createTranscriber({
    dir: config.transcriptsDir,
    prepare: (onProgress) => prepareTools(config.toolsDir, config.whisperCli, onProgress),
  }),
  remote,
});

export const url = `http://127.0.0.1:${config.port}`;
export const server = createServer(app, path.join(APP_ROOT, "build"), config.apiToken, remote.gate);

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
  console.log(`Projects: ${config.projectsDir}`);
  // Tailscale keeps Serve on across restarts, so remote access may already be on.
  void remote.refresh().then(() => {
    const status = remote.status();
    if (status.state === "on") console.log(`Remote access: ${status.url} (for ${status.owner})`);
  });
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
