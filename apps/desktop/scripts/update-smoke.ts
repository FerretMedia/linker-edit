// Tries the desktop app's updates end to end, against an update feed in a folder instead of GitHub. Linux only, where
// the app is one AppImage file that runs from anywhere; CI runs it under xvfb-run after laying out the app:
//
//   npm run build && cargo build --release -p linker-edit && node scripts/package.ts app
//   xvfb-run -a node scripts/update-smoke.ts        ($VPK is the vpk to run, "vpk" on PATH by default)
//
// 1. Packs release/app-linux-x64 twice with vpk, as an older and a newer release, and starts the older AppImage with
//    LINKER_EDIT_UPDATE_FEED pointing at the newer one's folder. The page (over the server's WebSocket, like the UI)
//    must see the update downloaded, with its notes.
// 2. Asks to restart to update, as the sidebar does. The app must quit, and Velopack must swap the AppImage for the
//    newer one and start it: the page then sees the newer version, and no record of an install that didn't apply.
//
// Everything goes in a temporary folder with its own config; the packages have an id of their own.
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import type { AppUpdate, ServerMessage } from "../shared/protocol.ts";

const REPO = path.join(import.meta.dirname, "..", "..", "..");
const APP = path.join(REPO, "release", "app-linux-x64");
const PACK_ID = "LinkerEditSmoke";
const OLD = "2026.101.1";
const NEW = "2026.101.2";
const NOTE = "Smoke test release";

function fail(message: string): never {
  console.error(`\n✗ ${message}`);
  process.exit(1);
}

if (process.platform !== "linux") fail("The update smoke test runs on Linux; CI runs it there.");
if (!fs.existsSync(path.join(APP, "linker-edit"))) fail(`No app in ${APP}. Run \`node scripts/package.ts app\` first.`);

const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-update-smoke-"));
console.log(`Working in ${tmp}`);

function pack(version: string, out: string) {
  const notes = path.join(tmp, `notes-${version}.md`);
  fs.writeFileSync(notes, `## What's Changed\n* ${NOTE} by @someone in https://github.com/FerretMedia/linker-edit/pull/1\n`);
  const args = ["pack", "--packId", PACK_ID, "--packVersion", version, "--packTitle", PACK_ID, "--packDir", APP];
  args.push("--mainExe", "linker-edit", "--channel", "linux-x64", "--outputDir", out, "--releaseNotes", notes, "--delta", "None");
  const result = spawnSync(process.env.VPK || "vpk", args, { stdio: "inherit" });
  if (result.status !== 0) fail(`vpk pack ${version} failed.`);
}
pack(OLD, path.join(tmp, "old"));
const feed = path.join(tmp, "feed");
pack(NEW, feed);

const appImage = path.join(tmp, "install", `${PACK_ID}.AppImage`);
fs.mkdirSync(path.dirname(appImage));
fs.copyFileSync(path.join(tmp, "old", `${PACK_ID}-linux-x64.AppImage`), appImage);
fs.chmodSync(appImage, 0o755);

const port = await new Promise<number>((resolve) => {
  const probe = http.createServer().listen(0, "127.0.0.1", () => {
    const { port } = probe.address() as AddressInfo;
    probe.close(() => resolve(port));
  });
});
const data = path.join(tmp, "data");
fs.mkdirSync(data);
const config = path.join(data, "linker-edit.config.json");
// No Resolve, and no speech engine download.
fs.writeFileSync(config, JSON.stringify({ port, resolveMcp: "", workspaceDir: path.join(tmp, "workspace"), projectsDir: path.join(tmp, "projects") }));
fs.writeFileSync(path.join(data, "settings.json"), JSON.stringify({ resolveHere: false }));
const record = path.join(data, "update-install.json");
const log = path.join(data, "logs", "linker-edit.log");

let first: ChildProcess | null = null;

/** Every process running from the temporary folder: the AppImage, its mount, and what Velopack started. */
function ours(): number[] {
  const found = spawnSync("pgrep", ["-f", tmp], { encoding: "utf8" }).stdout.split("\n").filter(Boolean).map(Number);
  return found.filter((pid) => pid !== process.pid);
}

function cleanUp() {
  for (const pid of ours()) {
    try {
      process.kill(pid, "SIGKILL");
    } catch {
      // Gone already.
    }
  }
  fs.rmSync(tmp, { recursive: true, force: true, maxRetries: 5, retryDelay: 500 });
}

function logs(): string {
  const read = (file: string) => (fs.existsSync(file) ? `--- ${file}\n${fs.readFileSync(file, "utf8").trim()}\n` : "");
  return [read(path.join(tmp, "first-run.log")), read(log)].join("");
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
        if (!update) return;
        last = update;
        if (predicate(update)) {
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
    if (child.exitCode !== null || child.signalCode !== null) return resolve();
    const timer = setTimeout(() => reject(new Error(`The app didn't quit within ${timeoutMs / 1000} seconds.`)), timeoutMs);
    child.once("exit", () => {
      clearTimeout(timer);
      resolve();
    });
  });

const output = fs.openSync(path.join(tmp, "first-run.log"), "w");
first = spawn(appImage, [], { env: { ...process.env, LINKER_EDIT_CONFIG: config, LINKER_EDIT_UPDATE_FEED: feed }, stdio: ["ignore", output, output] });
// A copy that quits at once would only show up as a timeout.
const quitEarly = (code: number | null) => {
  console.error(logs());
  cleanUp();
  fail(`The app quit before the update was downloaded (exit ${code}).`);
};
first.once("exit", quitEarly);
let socket: WebSocket | null = null;

await check("The page sees the update downloaded, with its notes", async () => {
  // The app looks 15 seconds after it starts.
  const found = await waitForUpdate((update) => update.status === "downloaded", 120_000);
  socket = found.socket;
  const { currentVersion, downloadedVersion, notes } = found.update;
  if (currentVersion !== OLD) throw new Error(`The app says it is ${currentVersion}, not ${OLD}.`);
  if (downloadedVersion !== NEW) throw new Error(`Downloaded ${downloadedVersion}, not ${NEW}.`);
  if (notes[0] !== NOTE) throw new Error(`The notes were ${JSON.stringify(notes)}.`);
});

await check("Restart to update quits, and Velopack starts the newer version", async () => {
  first!.off("exit", quitEarly);
  socket!.send(JSON.stringify({ type: "install_app_update" }));
  await exited(first!, 30_000);
  socket!.close();
  const found = await waitForUpdate((update) => update.currentVersion === NEW, 120_000);
  found.socket.close();
  if (found.update.notApplied !== null) throw new Error(`It says ${found.update.notApplied} didn't install.`);
  if (fs.existsSync(record)) throw new Error("The install record is still there after the install worked.");
});

cleanUp();
console.log("\nUpdates work: downloaded with their notes, installed on request, and the newer version started.");
process.exit(0);
