import { query } from "@anthropic-ai/claude-agent-sdk";
import * as path from "node:path";
import { releaseName } from "#shared/release-name.ts";
import { createApp } from "./app.ts";
import { APP_ROOT, loadConfigOrExit } from "./config.ts";
import { createHistory, readResolveSnapshot } from "./history/history.ts";
import { createServer } from "./http.ts";
import { createLinker } from "./linker.ts";
import { createPreviewRenderer } from "./preview/preview.ts";
import { fileProjectStore } from "./projects/store.ts";
import { createClaudeSession } from "./providers/claude.ts";
import { createCodexSession } from "./providers/codex.ts";
import { applyTimeline, backupProject } from "./resolve/versions.ts";
import { replayQuery } from "./providers/replay.ts";
import { createRemoteAccess } from "./remote.ts";
import { createResolveConnection } from "./resolve/connection.ts";
import { runResolveTool } from "./resolve/run.ts";
import { createResolveProjects } from "./resolveProjects.ts";
import { checkResolve } from "./resolveStatus.ts";
import { prepareWorkspace } from "./rules.ts";
import { fileSettings } from "./settings.ts";
import { prepareTools, toolsReady } from "./transcribe/tools.ts";
import { createTranscriber } from "./transcribe/transcriber.ts";
import { isNewer, latestVersion, VERSION } from "./updates.ts";

const config = loadConfigOrExit();

// Set to a recorded Claude turn (JSONL) to replay it instead of calling Claude: free turns for UI work.
const replay = process.env.LINKER_EDIT_REPLAY;
if (replay) console.log(`Replaying ${replay} instead of calling Claude`);

// The one connection to Resolve, through the MCP server Resolve Studio comes with.
const resolve = createResolveConnection({ command: config.resolveMcp.command });
process.on("exit", () => resolve.close());

const remote = createRemoteAccess({ port: config.port, servePort: config.tailscalePort });
// The browser comes back to this server after signing in at Linker.
const linker = createLinker({ ...config.linker, redirectUri: `http://127.0.0.1:${config.port}/api/linker/callback` });

const app = createApp({
  version: VERSION,
  store: fileProjectStore({ root: config.projectsDir, legacyDir: config.workspaceDir }),
  createSession: (provider, approvalMode, dir, resumeId) => {
    const projectConfig = { ...config, workspaceDir: dir };
    return provider === "codex"
      ? createCodexSession(projectConfig, approvalMode, resumeId)
      : createClaudeSession(projectConfig, approvalMode, { runQuery: replay ? replayQuery(replay) : query, resumeId });
  },
  checkResolve: () => checkResolve(resolve),
  // A replayed turn never touched Resolve, so there is nothing to save.
  createHistory: replay
    ? undefined
    : (dir, checkProject) => createHistory({ dir, readSnapshot: (scope) => readResolveSnapshot(resolve, scope), checkProject }),
  preview: createPreviewRenderer({ dir: config.previewDir, connection: resolve }),
  resolveProjects: createResolveProjects(resolve),
  resolveTools: (tool, input, dir, signal) => runResolveTool(resolve, tool, input, { dir, signal }),
  // A replayed turn never touches Resolve, so there is nothing to back up.
  backupProject: replay ? undefined : (dir) => backupProject(resolve, dir),
  applyTimeline: (dir, timelineId) => applyTimeline(resolve, dir, timelineId),
  prepareFolder: prepareWorkspace,
  transcriber: createTranscriber({
    dir: config.transcriptsDir,
    prepare: (onProgress, signal) => prepareTools(config.toolsDir, config.whisperCli, onProgress, signal),
    ready: () => toolsReady(config.toolsDir, config.whisperCli),
  }),
  remote,
  linker,
  settings: fileSettings(config.settingsFile),
});

export const url = `http://127.0.0.1:${config.port}`;
export const server = createServer(app, path.join(APP_ROOT, "build"), config.apiToken, remote, linker);

server.on("error", (error: NodeJS.ErrnoException) => {
  if (error.code === "EADDRINUSE") {
    console.error(`Port ${config.port} is already in use. Is another Linker-Edit server still running?`);
  } else {
    console.error(error);
  }
  process.exit(1);
});

server.listen(config.port, "127.0.0.1", () => {
  console.log(`Linker-Edit ${releaseName(VERSION)} on ${url}`);
  console.log(`Projects: ${config.projectsDir}`);
  // Tailscale keeps Serve on across restarts, so remote access may already be on.
  void remote.refresh().then(() => {
    const status = remote.status();
    if (status.state === "on") console.log(`Remote access: ${status.url} (for ${status.owner})`);
  });
  void app.refreshResolve();
  void linker.refresh();
  setInterval(() => {
    if (!app.busy()) void app.refreshResolve();
  }, 15_000);
});

// A checkout runs as 0.0.0 and has nothing to update to. The desktop app updates itself instead.
if (VERSION !== "0.0.0" && !process.env.LINKER_EDIT_DESKTOP) {
  const checkForUpdate = () =>
    void latestVersion().then((latest) => app.setUpdate(latest && isNewer(latest, VERSION) ? latest : null));
  checkForUpdate();
  setInterval(checkForUpdate, 24 * 60 * 60 * 1000);
}
