// The desktop window. It shows this computer's Linker-Edit, or the one on another of the user's computers over Tailscale.
// `linker-edit` starts the server and then this, with the server's address in LINKER_EDIT_URL. The installed app
// has no LINKER_EDIT_URL: it starts the server it ships with, and keeps itself up to date from GitHub releases
// (desktop/updater.ts).
import { execFile, execFileSync, fork, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  nativeTheme,
  shell,
  type BrowserWindowConstructorOptions,
  type IpcMainInvokeEvent,
  type TitleBarOverlayOptions,
} from "electron";
import type { Computers, DesktopToServer, ServerToDesktop } from "../shared/protocol.ts";
import { RELEASES_URL } from "../shared/release-name.ts";
import { legacyUpdate } from "./update-state.ts";
import { createUpdates, type Updates } from "./updater.ts";

const APP_DIR = path.join(import.meta.dirname, "..");
// `npm run dev:desktop` points it at the Vite dev server.
const given = process.env.LINKER_EDIT_URL ?? (app.isPackaged ? null : "http://localhost:5317");

let home = given ?? "";
// Only these origins may load in the window: this computer, and the computers it last listed.
const allowed = new Set<string>();
let server: ChildProcess | null = null;
// The server is being stopped on purpose, to install an update.
let stoppingServer = false;
let updates: Updates | null = null;
let quitting = false;

const originOf = (url: string) => {
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
};

async function listComputers(): Promise<Computers & { home: string }> {
  const response = await fetch(`${home}/api/computers`, { signal: AbortSignal.timeout(15_000) });
  if (!response.ok) throw new Error(`Linker-Edit answered ${response.status}.`);
  const computers = (await response.json()) as Computers;
  for (const computer of computers.others) allowed.add(new URL(computer.url).origin);
  return { ...computers, home };
}

const STARTING = `data:text/html;charset=utf-8,${encodeURIComponent(`<!doctype html>
<meta name="color-scheme" content="dark">
<body style="margin:0;height:100vh;display:grid;place-items:center;background:#0a0a0a;color:#a3a3a3;font:13px system-ui,sans-serif;-webkit-app-region:drag">
Starting Linker-Edit…`)}`;

// There's no system title bar, like T3 Code: the page's top headers are the title bar, and the system draws its
// window buttons over their corner. The page makes room for them (src/app.css); the preload says which kind.
const TITLE_BAR_HEIGHT = 52; // The headers' h-13.

// Windows and Linux draw minimize, maximize and close over the page; they take their color from the theme.
const overlay = (): TitleBarOverlayOptions => ({
  // Fully transparent misdraws on Linux.
  color: "#01000000",
  symbolColor: nativeTheme.shouldUseDarkColors ? "#f5f5f5" : "#27272a",
  height: TITLE_BAR_HEIGHT,
});

const titleBar = (): BrowserWindowConstructorOptions =>
  process.platform === "darwin"
    ? // The overlay tells the page where the traffic lights end (env(titlebar-area-x)), so the logo clears them.
      { titleBarStyle: "hiddenInset", trafficLightPosition: { x: 18, y: 19 }, titleBarOverlay: true }
    : { titleBarStyle: "hidden", titleBarOverlay: overlay() };

function createWindow(): BrowserWindow {
  const window = new BrowserWindow({
    width: 1320,
    height: 860,
    minWidth: 900,
    minHeight: 560,
    title: "Linker-Edit",
    backgroundColor: "#0a0a0a",
    autoHideMenuBar: true,
    ...titleBar(),
    webPreferences: { preload: path.join(APP_DIR, "desktop", "preload.cjs"), contextIsolation: true, sandbox: true },
  });
  const contents = window.webContents;
  const homeOrigin = () => originOf(home);

  // Links such as Tailscale's admin pages open in the browser, never in the app.
  contents.setWindowOpenHandler(({ url }) => {
    if (/^https?:\/\//.test(url)) void shell.openExternal(url);
    return { action: "deny" };
  });
  contents.on("will-navigate", (event, url) => {
    if (allowed.has(originOf(url) ?? "")) return;
    event.preventDefault();
    if (/^https?:\/\//.test(url)) void shell.openExternal(url);
  });

  const backHome = (title: string, detail: string) => {
    void dialog.showMessageBox(window, { type: "warning", message: title, detail });
    void window.loadURL(home);
  };
  contents.on("did-navigate", (_event, url, status) => {
    if (home && originOf(url) !== homeOrigin() && status >= 400) {
      backHome(
        "That computer didn't let you in.",
        "Remote access only lets in devices logged in to the same Tailscale account as that computer.",
      );
    }
  });
  contents.on("did-fail-load", (_event, _code, description, url, mainFrame) => {
    if (!mainFrame || !home) return;
    // The server may still be starting, as with `npm run dev:desktop`.
    if (originOf(url) === homeOrigin()) setTimeout(() => void window.loadURL(home), 1000);
    else backHome("Couldn't reach that computer.", `Is it on, with Linker-Edit running and remote access on? (${description})`);
  });

  const recolor = () => {
    if (process.platform !== "darwin" && !window.isDestroyed()) window.setTitleBarOverlay(overlay());
  };
  nativeTheme.on("updated", recolor);
  window.on("closed", () => nativeTheme.off("updated", recolor));

  void window.loadURL(home || STARTING);
  return window;
}

/** Shows `url` as this computer's Linker-Edit. */
function goHome(window: BrowserWindow, url: string) {
  home = url;
  allowed.add(new URL(url).origin);
  void window.loadURL(url);
}

/**
 * The PATH of the user's login shell. Apps opened from the Dock or a launcher get a bare PATH, without where
 * Homebrew, uv and the agents' tools live. Windows apps get the user's PATH already.
 */
function loginShellPath(): Promise<string | undefined> {
  if (process.platform === "win32") return Promise.resolve(undefined);
  const shellPath = process.env.SHELL || (process.platform === "darwin" ? "/bin/zsh" : "/bin/sh");
  return new Promise((resolve) => {
    execFile(shellPath, ["-ilc", 'printf "\\n@@%s@@" "$PATH"'], { timeout: 5000, encoding: "utf8" }, (_error, stdout) =>
      resolve(/@@(.+)@@/.exec(stdout ?? "")?.[1]),
    );
  });
}

/** Starts the server this app ships with, run as Node, and resolves with its address once it's listening. */
async function startServer(log: string): Promise<string> {
  fs.mkdirSync(path.dirname(log), { recursive: true });
  const output = fs.createWriteStream(log);
  const env: NodeJS.ProcessEnv = { ...process.env, ELECTRON_RUN_AS_NODE: "1", LINKER_EDIT_DESKTOP: "1" };
  const shellPath = await loginShellPath();
  if (shellPath) env.PATH = shellPath;
  const child = fork(path.join(APP_DIR, "dist", "linker-edit.js"), ["--serve"], {
    env,
    stdio: ["ignore", "pipe", "pipe", "ipc"],
    // Its own process group, so stopping it stops what it started too (stopServer). Windows goes by process tree.
    detached: process.platform !== "win32",
  });
  server = child;
  child.stdout?.pipe(output);
  child.stderr?.pipe(output);
  return new Promise((resolve, reject) => {
    child.on("message", (message: ServerToDesktop) => {
      switch (message.type) {
        case "ready":
          if (updates) sendToServer({ type: "update_state", state: updates.state() });
          resolve(message.url);
          break;
        case "check_update":
          void updates?.check();
          break;
        case "install_update":
          void updates?.install();
          break;
      }
    });
    child.on("error", reject);
    child.on("exit", (code) => {
      server = null;
      reject(new Error(`It stopped (exit ${code}).`));
    });
  });
}

function sendToServer(message: DesktopToServer) {
  if (server?.connected) server.send(message);
}

/**
 * Stops the server and every process it started: the agents (claude, codex), ffmpeg, and the helpers that run
 * this app's own binary. Any of them still running keeps the installer from replacing its files.
 */
function killServer(child: ChildProcess, signal: "SIGTERM" | "SIGKILL" = "SIGTERM") {
  if (child.pid === undefined) return;
  try {
    if (process.platform === "win32") {
      execFileSync("taskkill", ["/pid", String(child.pid), "/T", "/F"], { stdio: "ignore", windowsHide: true });
    } else {
      process.kill(-child.pid, signal);
    }
  } catch {
    // Already gone, or only the server is left.
    child.kill(signal);
  }
}

async function stopServer() {
  const child = server;
  if (!child) return;
  stoppingServer = true;
  const exited = new Promise<void>((resolve) => {
    if (child.exitCode !== null || child.signalCode !== null) resolve();
    else child.once("exit", () => resolve());
  });
  killServer(child);
  await Promise.race([exited, new Promise((resolve) => setTimeout(resolve, 5000))]);
  // What it started may outlive it on macOS and Linux; they share its process group.
  if (process.platform !== "win32") killServer(child, "SIGKILL");
}

/** Starts the server and shows it. Quits when it can't start, or when it stops without being asked to. */
async function runServer(window: BrowserWindow, log: string): Promise<boolean> {
  stoppingServer = false;
  try {
    goHome(window, await startServer(log));
  } catch (error) {
    dialog.showErrorBox(
      "Linker-Edit couldn't start",
      `${error instanceof Error ? error.message : String(error)}\n\n${tail(log)}\n\nThe full log is in ${log}`,
    );
    app.quit();
    return false;
  }
  server?.on("exit", () => {
    if (app.isReady() && !quitting && !stoppingServer) {
      dialog.showErrorBox("Linker-Edit stopped", `${tail(log)}\n\nThe full log is in ${log}`);
      app.quit();
    }
  });
  return true;
}

/** The end of the server's log, for error messages. */
function tail(log: string): string {
  try {
    return fs.readFileSync(log, "utf8").trim().split("\n").slice(-12).join("\n");
  } catch {
    return "";
  }
}

/**
 * Keeps the app up to date from GitHub releases. The server shows the update to every page and asks to install it
 * once nothing is busy with Resolve. LINKER_EDIT_UPDATE_FEED points it at another feed, for scripts/update-smoke.ts.
 */
async function keepUpToDate(window: BrowserWindow, serverLog: string) {
  const { autoUpdater } = (await import("electron-updater")).default;
  const feed = process.env.LINKER_EDIT_UPDATE_FEED;
  if (feed) {
    autoUpdater.setFeedURL({ provider: "generic", url: feed });
    // A test feed has no blockmaps to download only what changed.
    autoUpdater.disableDifferentialDownload = true;
  }
  const record = path.join(app.getPath("userData"), "update-install.json");
  const log = path.join(app.getPath("logs"), "updates.log");
  updates = createUpdates({
    updater: autoUpdater,
    currentVersion: app.getVersion(),
    // An unsigned Mac app can't replace itself, so macOS only says where to get the new version.
    canInstall: process.platform !== "darwin",
    releasesUrl: RELEASES_URL,
    readRecord() {
      try {
        return JSON.parse(fs.readFileSync(record, "utf8")) as unknown;
      } catch {
        return null;
      }
    },
    writeRecord(value) {
      try {
        if (value) fs.writeFileSync(record, JSON.stringify(value));
        else fs.rmSync(record, { force: true });
      } catch (error) {
        console.error(`Couldn't keep the update record: ${error instanceof Error ? error.message : String(error)}`);
      }
    },
    stopServer,
    resumeServer: async () => void (await runServer(window, serverLog)),
    onState(state) {
      sendToServer({ type: "update_state", state });
      // Pages of older Linker-Edits, on the user's other computers, still listen here.
      if (!window.isDestroyed()) window.webContents.send("update", legacyUpdate(state));
    },
    log(line) {
      console.log(`Update: ${line}`);
      fs.appendFile(log, `${new Date().toISOString()} ${line}\n`, () => {});
    },
  });
  sendToServer({ type: "update_state", state: updates.state() });
  updates.start();
}

// A second copy only brings the first one's window forward (second-instance below); it starts no server of its
// own, which would also wipe the running one's log.
const primary = app.requestSingleInstanceLock();
if (!primary) app.quit();

void app.whenReady().then(async () => {
  if (!primary) return;
  if (home) allowed.add(new URL(home).origin);
  const window = createWindow();
  ipcMain.handle("computers", () => listComputers());
  ipcMain.handle("open", (_event, url: unknown) => {
    if (typeof url === "string" && allowed.has(originOf(url) ?? "")) void window.loadURL(url);
  });
  // What pages of older Linker-Edits ask; newer ones get the update from the server.
  ipcMain.handle("update", () => (updates ? legacyUpdate(updates.state()) : null));
  ipcMain.handle("install-update", () => sendToServer({ type: "install_requested" }));
  // A folder picked here only means something to this computer's own server, not to another computer's page.
  const fromHome = (event: IpcMainInvokeEvent) => Boolean(home) && originOf(event.senderFrame?.url ?? "") === originOf(home);
  ipcMain.handle("can-pick-folder", (event) => fromHome(event));
  ipcMain.handle("pick-folder", async (event, defaultPath: unknown) => {
    if (!fromHome(event)) return null;
    const result = await dialog.showOpenDialog(window, {
      properties: ["openDirectory", "createDirectory"],
      ...(typeof defaultPath === "string" && defaultPath ? { defaultPath } : {}),
    });
    return result.canceled ? null : (result.filePaths[0] ?? null);
  });
  app.on("second-instance", () => {
    if (window.isMinimized()) window.restore();
    window.focus();
  });
  if (given) return;

  const log = path.join(app.getPath("logs"), "server.log");
  if (await runServer(window, log)) void keepUpToDate(window, log);
});

app.on("before-quit", () => {
  quitting = true;
});
app.on("will-quit", () => {
  // electron-updater installs a downloaded release once the app has quit, so nothing may hold the app's files.
  updates?.quitting();
  if (server) killServer(server);
});
app.on("window-all-closed", () => app.quit());
