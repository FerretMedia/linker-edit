// The desktop window. `linker-edit` starts the server and then this, with the server's address in LINKER_EDIT_URL.
// It shows this computer's Linker-Edit, or the one on another of the user's computers over Tailscale.
import { app, BrowserWindow, dialog, ipcMain, shell } from "electron";
import * as path from "node:path";
import type { Computers } from "../shared/protocol.ts";

// `npm run dev:desktop` points it at the Vite dev server.
const home = process.env.LINKER_EDIT_URL ?? "http://localhost:5317";
const homeOrigin = new URL(home).origin;
// Only these origins may load in the window: this computer, and the computers it last listed.
const allowed = new Set([homeOrigin]);

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

function createWindow(): BrowserWindow {
  const window = new BrowserWindow({
    width: 1320,
    height: 860,
    minWidth: 900,
    minHeight: 560,
    title: "Linker-Edit",
    backgroundColor: "#0a0a0a",
    autoHideMenuBar: true,
    webPreferences: { preload: path.join(import.meta.dirname, "..", "desktop", "preload.cjs"), contextIsolation: true, sandbox: true },
  });
  const contents = window.webContents;
  // The app needs no camera, microphone, location or notifications, not even on another computer's page.
  const PERMISSIONS = new Set(["fullscreen", "clipboard-sanitized-write"]);
  contents.session.setPermissionRequestHandler((_contents, permission, callback) => callback(PERMISSIONS.has(permission)));

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
    if (originOf(url) !== homeOrigin && status >= 400) {
      backHome(
        "That computer didn't let you in.",
        "Remote access only lets in devices logged in to the same Tailscale account as that computer.",
      );
    }
  });
  contents.on("did-fail-load", (_event, _code, description, url, mainFrame) => {
    if (!mainFrame) return;
    // The server may still be starting, as with `npm run dev:desktop`.
    if (originOf(url) === homeOrigin) setTimeout(() => void window.loadURL(home), 1000);
    else backHome("Couldn't reach that computer.", `Is it on, with Linker-Edit running and remote access on? (${description})`);
  });

  void window.loadURL(home);
  return window;
}

if (!app.requestSingleInstanceLock()) app.quit();

void app.whenReady().then(() => {
  const window = createWindow();
  ipcMain.handle("computers", () => listComputers());
  ipcMain.handle("open", (_event, url: unknown) => {
    if (typeof url === "string" && allowed.has(originOf(url) ?? "")) void window.loadURL(url);
  });
  app.on("second-instance", () => {
    if (window.isMinimized()) window.restore();
    window.focus();
  });
});

app.on("window-all-closed", () => app.quit());
