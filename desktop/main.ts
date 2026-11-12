// The desktop window. It shows this computer's Linker-Edit, or the one on another of the user's computers over Tailscale.
// `linker-edit` starts the server and then this, with the server's address in LINKER_EDIT_URL. The installed app
// has no LINKER_EDIT_URL: it starts the server it ships with, and keeps itself up to date from GitHub releases.
import { execFile, fork, type ChildProcess } from "node:child_process";
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
import type { Computers, DesktopUpdate } from "../shared/protocol.ts";

const APP_DIR = path.join(import.meta.dirname, "..");
const RELEASES_URL = "https://github.com/FerretMedia/linker-edit/releases/latest";
// `npm run dev:desktop` points it at the Vite dev server.
const given = process.env.LINKER_EDIT_URL ?? (app.isPackaged ? null : "http://localhost:5317");

let home = given ?? "";
// Only these origins may load in the window: this computer, and the computers it last listed.
const allowed = new Set<string>();
let server: ChildProcess | null = null;
let update: DesktopUpdate | null = null;
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
  const child = fork(path.join(APP_DIR, "dist", "linker-edit.js"), ["--serve"], { env, stdio: ["ignore", "pipe", "pipe", "ipc"] });
  server = child;
  child.stdout?.pipe(output);
  child.stderr?.pipe(output);
  return new Promise((resolve, reject) => {
    child.on("message", (message: { type?: string; url?: string }) => {
      if (message.type === "ready" && message.url) resolve(message.url);
    });
    child.on("error", reject);
    child.on("exit", (code) => {
      server = null;
      reject(new Error(`It stopped (exit ${code}).`));
    });
  });
}

/** The end of the server's log, for error messages. */
function tail(log: string): string {
  try {
    return fs.readFileSync(log, "utf8").trim().split("\n").slice(-12).join("\n");
  } catch {
    return "";
  }
}

/** Downloads new releases in the background and tells the page when one is ready. */
async function keepUpToDate(window: BrowserWindow) {
  const { autoUpdater } = (await import("electron-updater")).default;
  const announce = (next: DesktopUpdate) => {
    update = next;
    if (!window.isDestroyed()) window.webContents.send("update", next);
  };
  // An unsigned Mac app can't replace itself, so macOS only says where to get the new version.
  autoUpdater.autoDownload = process.platform !== "darwin";
  autoUpdater.on("update-available", (info) => {
    if (!autoUpdater.autoDownload) announce({ version: info.version, state: "download", url: RELEASES_URL });
  });
  autoUpdater.on("update-downloaded", (info) => announce({ version: info.version, state: "ready" }));
  autoUpdater.on("error", (error) => console.error(`Update check failed: ${error.message}`));
  const check = () => void autoUpdater.checkForUpdates().catch(() => {});
  check();
  setInterval(check, 6 * 60 * 60 * 1000);
  ipcMain.handle("install-update", (event) => {
    if (update?.state === "ready" && originOf(event.senderFrame?.url ?? "") === originOf(home)) autoUpdater.quitAndInstall();
  });
}

if (!app.requestSingleInstanceLock()) app.quit();

void app.whenReady().then(async () => {
  if (home) allowed.add(new URL(home).origin);
  const window = createWindow();
  ipcMain.handle("computers", () => listComputers());
  ipcMain.handle("open", (_event, url: unknown) => {
    if (typeof url === "string" && allowed.has(originOf(url) ?? "")) void window.loadURL(url);
  });
  ipcMain.handle("update", () => update);
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
  try {
    goHome(window, await startServer(log));
  } catch (error) {
    dialog.showErrorBox(
      "Linker-Edit couldn't start",
      `${error instanceof Error ? error.message : String(error)}\n\n${tail(log)}\n\nThe full log is in ${log}`,
    );
    app.quit();
    return;
  }
  server?.on("exit", () => {
    if (app.isReady() && !quitting) {
      dialog.showErrorBox("Linker-Edit stopped", `${tail(log)}\n\nThe full log is in ${log}`);
      app.quit();
    }
  });
  void keepUpToDate(window);
});

app.on("before-quit", () => {
  quitting = true;
});
app.on("will-quit", () => server?.kill());
app.on("window-all-closed", () => app.quit());
