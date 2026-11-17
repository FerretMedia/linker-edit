import { query } from "@anthropic-ai/claude-agent-sdk";
import * as path from "node:path";
import { releaseName } from "#shared/release-name.ts";
import type { DesktopToServer } from "#shared/protocol.ts";
import { createApp } from "./app.ts";
import { createAppUpdates } from "./appUpdate.ts";
import { fileCollectionStore } from "./collections/store.ts";
import { APP_ROOT, loadConfigOrExit } from "./config.ts";
import { createHistory, readResolveSnapshot } from "./history/history.ts";
import { createServer } from "./http.ts";
import { fileLibraryStore } from "./library/folders.ts";
import { createLibrary } from "./library/library.ts";
import { ffmpegProbe } from "./library/scan.ts";
import { createLinker } from "./linker.ts";
import { prepareCloudflared } from "./connect/cloudflared.ts";
import { createConnect } from "./connect/connect.ts";
import { createSessions } from "./connect/sessions.ts";
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
import { findFfmpeg, prepareTools, toolsReady } from "./transcribe/tools.ts";
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
// Linker Connect: up to 3 of your edit computers at their own addresses at Linker, through Cloudflare Tunnels.
const connectSessions = createSessions({ file: config.connect.sessionsFile });
const connect = createConnect({
  file: config.connect.file,
  port: config.port,
  linker,
  appUrl: config.linker.appUrl,
  sessions: connectSessions,
  prepare: (onPercent) => prepareCloudflared(config.toolsDir, config.connect.cloudflared, onPercent),
});

// The desktop app started this server and talks to it over IPC: its own updates go through here.
const appUpdates =
  process.env.LINKER_EDIT_DESKTOP && process.send
    ? createAppUpdates({ send: (message) => process.send?.(message), busy: () => app.busy() })
    : undefined;
if (appUpdates) process.on("message", (message: DesktopToServer) => appUpdates.receive(message));

const app = createApp({
  version: VERSION,
  store: fileProjectStore({ root: config.projectsDir, legacyDir: config.workspaceDir }),
  // Workspaces and series live next to the config: ~/LinkerEdit/collections.json and collections/.
  collections: fileCollectionStore({ root: path.dirname(config.configFile) }),
  // Codex reads the project's rules from AGENTS.md, written before each turn (prepareFolder).
  createSession: (provider, approvalMode, dir, resumeId, rules) => {
    const projectConfig = { ...config, workspaceDir: dir };
    return provider === "codex"
      ? createCodexSession(projectConfig, approvalMode, resumeId)
      : createClaudeSession(projectConfig, approvalMode, { runQuery: replay ? replayQuery(replay) : query, resumeId, rules });
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
  connect,
  settings: fileSettings(config.settingsFile),
  setup: {
    configFile: config.configFile,
    folders: { projects: config.projectsDir, previews: config.previewDir, transcripts: config.transcriptsDir, tools: config.toolsDir },
    resolveMcp: config.resolveMcp.command,
  },
  appUpdates,
  library: createLibrary({ store: fileLibraryStore(config.libraryFile), cacheDir: config.libraryCacheDir, probe: ffmpegProbe(findFfmpeg) }),
});

export const url = `http://127.0.0.1:${config.port}`;
export const server = createServer(app, path.join(APP_ROOT, "build"), config.apiToken, remote, linker, { ...connect, sessions: connectSessions });

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
  app.prepareTranscription();
  // Picks up files added to library folders while Linker-Edit was closed; unchanged files aren't measured again.
  void app.scanLibrary();
  void linker.refresh();
  // Linker Connect stays on across restarts, like Tailscale's remote access.
  connect.start();
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
