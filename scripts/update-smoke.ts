// Tries the desktop app's updates end to end, against an update feed on this computer instead of GitHub:
//
//   npm run build && node scripts/build-desktop.ts --dir && node scripts/update-smoke.ts
//
// 1. Starts the unpacked app from release/ with LINKER_EDIT_UPDATE_FEED pointing at a feed that offers the next
//    version. The page (over the server's WebSocket, like the UI) must see it downloaded, with its notes.
// 2. Asks to restart to update, as the sidebar does. The app must stop its server and quit.
// 3. The "installer" is made-up bytes that can't run, so the app starts again on the old version: it must say the
//    update didn't install.
//
// On macOS the app can't install by itself (unsigned), so only step 1 runs and it must offer a download link.
// On Linux run it under xvfb-run. It leaves nothing behind but what Electron keeps in the app's own folders.
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import { createHash, randomBytes } from "node:crypto";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import type { AppUpdate, ServerMessage } from "../shared/protocol.ts";

const ROOT = path.join(import.meta.dirname, "..");
const RELEASE = path.join(ROOT, "release");
const NOTE = "Smoke test release";

function fail(message: string): never {
  console.error(`\n✗ ${message}`);
  process.exit(1);
}

/** The unpacked app electron-builder laid out, its binary, and the version it carries. */
function findApp(): { dir: string; binary: string; version: string; resources: string } {
  const candidates =
    process.platform === "win32"
      ? [["win-unpacked", "Linker-Edit.exe"]]
      : process.platform === "darwin"
        ? [
            ["mac-arm64", "Linker-Edit.app/Contents/MacOS/Linker-Edit"],
            ["mac", "Linker-Edit.app/Contents/MacOS/Linker-Edit"],
          ]
        : [
            ["linux-unpacked", "linker-edit"],
            ["linux-arm64-unpacked", "linker-edit"],
          ];
  for (const [dir, binary] of candidates) {
    const file = path.join(RELEASE, dir!, binary!);
    if (!fs.existsSync(file)) continue;
    const resources = process.platform === "darwin" ? path.join(RELEASE, dir!, "Linker-Edit.app/Contents/Resources") : path.join(RELEASE, dir!, "resources");
    const { version } = JSON.parse(fs.readFileSync(path.join(resources, "app", "package.json"), "utf8")) as { version: string };
    return { dir: path.join(RELEASE, dir!), binary: file, version, resources };
  }
  fail(`No unpacked app in ${RELEASE}. Run \`npm run build && node scripts/build-desktop.ts --dir\` first.`);
}

/** Where Electron keeps the app's settings and electron-updater its downloads, on each platform. */
function appFolders() {
  const home = os.homedir();
  switch (process.platform) {
    case "win32":
      return {
        userData: path.join(process.env.APPDATA ?? path.join(home, "AppData", "Roaming"), "Linker-Edit"),
        updaterCache: path.join(process.env.LOCALAPPDATA ?? path.join(home, "AppData", "Local"), "linker-edit-updater"),
      };
    case "darwin":
      return {
        userData: path.join(home, "Library", "Application Support", "Linker-Edit"),
        updaterCache: path.join(home, "Library", "Caches", "linker-edit-updater"),
      };
    default:
      return {
        userData: path.join(process.env.XDG_CONFIG_HOME ?? path.join(home, ".config"), "Linker-Edit"),
        updaterCache: path.join(process.env.XDG_CACHE_HOME ?? path.join(home, ".cache"), "linker-edit-updater"),
      };
  }
}

const app = findApp();
const folders = appFolders();
const [major, minor, patch] = app.version.split(".").map(Number) as [number, number, number];
const next = `${major}.${minor}.${patch + 1}`;
const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-update-smoke-"));
const record = path.join(folders.userData, "update-install.json");
console.log(`App ${app.binary} (${app.version}); the feed offers ${next}. Working in ${tmp}`);

// The feed: electron-updater's generic provider reads latest*.yml next to the files it lists.
const feedDir = path.join(tmp, "feed");
fs.mkdirSync(feedDir);
const ext = process.platform === "win32" ? "exe" : process.platform === "darwin" ? "zip" : "AppImage";
const file = `Linker-Edit-${next}.${ext}`;
// An "installer" that runs and does nothing. Bytes that can't run would make electron-updater try again through
// elevate.exe on Windows, which asks for admin rights.
const bytes =
  process.platform === "win32"
    ? fs.readFileSync(path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "whoami.exe"))
    : process.platform === "linux"
      ? Buffer.from(`#!/bin/sh\nexit 0\n# ${randomBytes(1024 * 1024).toString("hex")}\n`)
      : randomBytes(2 * 1024 * 1024);
fs.writeFileSync(path.join(feedDir, file), bytes);
const sha512 = createHash("sha512").update(bytes).digest("base64");
const channel = process.platform === "win32" ? "latest" : process.platform === "darwin" ? "latest-mac" : process.arch === "arm64" ? "latest-linux-arm64" : "latest-linux";
fs.writeFileSync(
  path.join(feedDir, `${channel}.yml`),
  [
    `version: ${next}`,
    "files:",
    `  - url: ${file}`,
    `    sha512: ${sha512}`,
    `    size: ${bytes.length}`,
    `path: ${file}`,
    `sha512: ${sha512}`,
    `releaseDate: '${new Date().toISOString()}'`,
    `releaseNotes: '<ul><li>${NOTE} by @someone in #1</li></ul>'`,
    "",
  ].join("\n"),
);
const feed = http.createServer((req, res) => {
  const name = path.basename(decodeURIComponent(new URL(req.url ?? "/", "http://localhost").pathname));
  const target = path.join(feedDir, name);
  if (!name || !fs.existsSync(target)) return void res.writeHead(404).end();
  res.writeHead(200, { "content-length": fs.statSync(target).size });
  fs.createReadStream(target).pipe(res);
});
await new Promise<void>((resolve) => feed.listen(0, "127.0.0.1", resolve));
const feedUrl = `http://127.0.0.1:${(feed.address() as AddressInfo).port}/`;

const port = await new Promise<number>((resolve) => {
  const probe = http.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address() as AddressInfo;
    probe.close(() => resolve(port));
  });
});
const config = path.join(tmp, "config.json");
fs.writeFileSync(config, JSON.stringify({ port, workspaceDir: path.join(tmp, "workspace"), projectsDir: path.join(tmp, "projects") }));

// Installers carry app-update.yml, which says where the updater keeps its downloads; an unpacked build doesn't.
const updateYml = path.join(app.resources, "app-update.yml");
const madeUpdateYml = !fs.existsSync(updateYml);
if (madeUpdateYml) fs.writeFileSync(updateYml, `provider: generic\nurl: ${feedUrl}\nupdaterCacheDirName: linker-edit-updater\n`);

// A downloaded update or install record from an earlier run would skip what this tries.
fs.rmSync(record, { force: true });
fs.rmSync(folders.updaterCache, { recursive: true, force: true });

let running: ChildProcess | null = null;

function launch(label: string): ChildProcess {
  const log = fs.openSync(path.join(tmp, `${label}.log`), "w");
  const env: NodeJS.ProcessEnv = { ...process.env, LINKER_EDIT_UPDATE_FEED: feedUrl, LINKER_EDIT_CONFIG: config };
  delete env.ELECTRON_RUN_AS_NODE;
  if (process.platform === "linux") {
    // The AppImage updater only runs inside an AppImage, and installs over this file.
    env.APPIMAGE = path.join(tmp, "Linker-Edit.AppImage");
    fs.writeFileSync(env.APPIMAGE, "");
  }
  const args = process.platform === "linux" ? ["--no-sandbox"] : [];
  running = spawn(app.binary, args, { env, stdio: ["ignore", log, log] });
  return running;
}

function stop(child: ChildProcess | null) {
  if (!child || child.exitCode !== null || child.pid === undefined) return;
  if (process.platform === "win32") spawnSync("taskkill", ["/pid", String(child.pid), "/T", "/F"], { stdio: "ignore" });
  else child.kill("SIGKILL");
}

function logs(): string {
  const read = (file: string) => (fs.existsSync(file) ? `--- ${file}\n${fs.readFileSync(file, "utf8").trim()}\n` : "");
  const logDir = process.platform === "darwin" ? path.join(os.homedir(), "Library", "Logs", "Linker-Edit") : path.join(folders.userData, "logs");
  return [
    ...fs.readdirSync(tmp).filter((name) => name.endsWith(".log")).map((name) => read(path.join(tmp, name))),
    read(path.join(logDir, "updates.log")),
    read(path.join(logDir, "server.log")),
  ].join("");
}

function cleanUp() {
  stop(running);
  feed.close();
  fs.rmSync(record, { force: true });
  fs.rmSync(folders.updaterCache, { recursive: true, force: true });
  if (madeUpdateYml) fs.rmSync(updateYml, { force: true });
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
}

async function check(step: string, run: () => Promise<void>) {
  process.stdout.write(`${step}… `);
  try {
    await run();
    console.log("ok");
  } catch (error) {
    console.log("failed");
    console.error(logs());
    cleanUp();
    fail(`${step}: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/** Connects to the app's server like the UI, retrying while it starts, and waits for an update state. */
async function waitForUpdate(predicate: (update: AppUpdate) => boolean, timeoutMs: number): Promise<{ update: AppUpdate; socket: WebSocket }> {
  const deadline = Date.now() + timeoutMs;
  let last: AppUpdate | null = null;
  while (Date.now() < deadline) {
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`);
    const result = await new Promise<AppUpdate | "closed">((resolve) => {
      const timer = setTimeout(() => resolve("closed"), Math.max(0, deadline - Date.now()));
      socket.onmessage = (event) => {
        const message = JSON.parse(String(event.data)) as ServerMessage;
        const update = message.type === "snapshot" ? message.state.appUpdate : message.type === "meta" ? message.meta.appUpdate : undefined;
        if (update === undefined) return;
        last = update;
        if (update && predicate(update)) {
          clearTimeout(timer);
          resolve(update);
        }
      };
      socket.onclose = socket.onerror = () => {
        clearTimeout(timer);
        resolve("closed");
      };
    });
    if (result !== "closed") return { update: result, socket };
    socket.close();
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
  throw new Error(`Timed out. The page last saw: ${JSON.stringify(last)}`);
}

const exited = (child: ChildProcess, timeoutMs: number) =>
  new Promise<void>((resolve, reject) => {
    if (child.exitCode !== null) return resolve();
    const timer = setTimeout(() => reject(new Error(`The app didn't quit within ${timeoutMs / 1000} seconds.`)), timeoutMs);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });

/** Processes still running from the app's folder; the installer can't replace their files. */
function leftovers(): string[] {
  if (process.platform === "win32") {
    const script = `Get-Process | Where-Object { $_.Path -and $_.Path.StartsWith('${app.dir.replace(/'/g, "''")}', 'OrdinalIgnoreCase') } | ForEach-Object { "$($_.Id) $($_.Path)" }`;
    return spawnSync("powershell", ["-NoProfile", "-Command", script], { encoding: "utf8" }).stdout.split(/\r?\n/).filter(Boolean);
  }
  return spawnSync("pgrep", ["-af", app.dir], { encoding: "utf8" }).stdout.split("\n").filter(Boolean);
}

const first = launch("first-run");
let socket: WebSocket | null = null;

if (process.platform === "darwin") {
  await check("The update is offered as a download", async () => {
    const found = await waitForUpdate((update) => update.status === "available", 120_000);
    if (found.update.availableVersion !== next || !found.update.downloadUrl) throw new Error(JSON.stringify(found.update));
    found.socket.close();
  });
  cleanUp();
  console.log("\nThe update was offered. macOS apps without a Developer ID can't install by themselves, so that's all here.");
  process.exit(0);
}

await check("The page sees the update downloaded, with its notes", async () => {
  // The app looks 15 seconds after it starts.
  const found = await waitForUpdate((update) => update.status === "downloaded", 120_000);
  socket = found.socket;
  const { downloadedVersion, notes } = found.update;
  if (downloadedVersion !== next) throw new Error(`Downloaded ${downloadedVersion}, not ${next}.`);
  if (notes[0] !== NOTE) throw new Error(`The notes were ${JSON.stringify(notes)}.`);
});

await check("Restart to update stops the server and quits", async () => {
  socket!.send(JSON.stringify({ type: "install_app_update" }));
  await exited(first, 30_000);
  // The installer the app started may still be starting; what matters is that nothing of the app holds its files.
  const left = leftovers().filter((line) => !line.includes(file));
  if (left.length) throw new Error(`Still running from the app's folder:\n${left.join("\n")}`);
  const kept = JSON.parse(fs.readFileSync(record, "utf8")) as { from: string; to: string };
  if (kept.from !== app.version || kept.to !== next) throw new Error(`The install record says ${JSON.stringify(kept)}.`);
});

await check("Started again on the old version, it says the update didn't install", async () => {
  launch("second-run");
  const found = await waitForUpdate((update) => update.notApplied !== null, 60_000);
  found.socket.close();
  if (found.update.notApplied !== next) throw new Error(`It says ${found.update.notApplied} didn't install.`);
});

cleanUp();
console.log("\nUpdates work: downloaded, installed on request with the server stopped, and a failed install is reported.");
process.exit(0);
