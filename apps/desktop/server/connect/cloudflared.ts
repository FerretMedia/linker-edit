import { execFile, spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { download, findOnPath, type Download } from "../transcribe/tools.ts";

/**
 * cloudflared keeps this computer's Linker Connect tunnel open, as it does for T3 Connect. Pinned and checked
 * by hash like the speech engine, so every computer runs the same tested release. One the user installed
 * (on PATH, or `cloudflared` in the config) is their choice and comes first.
 */
export const CLOUDFLARED_VERSION = "2026.10.0";
const RELEASE = `https://github.com/cloudflare/cloudflared/releases/download/${CLOUDFLARED_VERSION}`;

type Build = Download & { tgz?: true };

/** By platform and processor. Windows on Arm runs the x64 build. The hashes are GitHub's asset digests. */
export const CLOUDFLARED_BUILDS: Record<string, Build> = {
  "win32-x64": {
    url: `${RELEASE}/cloudflared-windows-amd64.exe`,
    sha256: "86aee4017b26625cee8484c113558f48effa4cd47f7aa05fcf425604e5d2b23c",
    size: 55_365_048,
  },
  "linux-x64": {
    url: `${RELEASE}/cloudflared-linux-amd64`,
    sha256: "d33ff2d14475178d2012c2c56beba87389ac5ded27649519f198a7d3134a99db",
    size: 40_129_756,
  },
  "linux-arm64": {
    url: `${RELEASE}/cloudflared-linux-arm64`,
    sha256: "e6422b9d4f72d3194bc5a38676f13667c06666523217b842a877d72a80b5ac08",
    size: 37_687_584,
  },
  "darwin-x64": {
    url: `${RELEASE}/cloudflared-darwin-amd64.tgz`,
    sha256: "903845b81828c8cb3c5d13d816a2de71c06a3da5785469df8eb0e1b736d92f9f",
    size: 21_741_581,
    tgz: true,
  },
  "darwin-arm64": {
    url: `${RELEASE}/cloudflared-darwin-arm64.tgz`,
    sha256: "a2f79ff7b9420aa537d74af239f376da170bbabeb529aec416002adac6a72e70",
    size: 19_809_074,
    tgz: true,
  },
};

export function buildFor(platform: NodeJS.Platform, arch: NodeJS.Architecture): Build | null {
  const key = platform === "win32" ? "win32-x64" : `${platform}-${arch}`;
  return CLOUDFLARED_BUILDS[key] ?? null;
}

/** Finds or downloads cloudflared, about 20 to 55 MB once. Answers the file to run. */
export async function prepareCloudflared(dir: string, override: string | undefined, onPercent: (percent: number) => void): Promise<string> {
  if (override) {
    if (!fs.existsSync(override)) throw new Error(`cloudflared isn't at ${override}. Fix "cloudflared" in linker-edit.config.json.`);
    return override;
  }
  const onPath = findOnPath("cloudflared");
  if (onPath) return onPath;
  const build = buildFor(process.platform, process.arch);
  if (!build) throw new Error("There's no cloudflared for this computer. Install it so it's on PATH, then try again.");
  const home = path.join(dir, `cloudflared-${CLOUDFLARED_VERSION}`);
  const file = path.join(home, process.platform === "win32" ? "cloudflared.exe" : "cloudflared");
  if (fs.existsSync(file)) return file;
  if (!build.tgz) {
    await download(build, file, onPercent);
  } else {
    const archive = path.join(dir, `cloudflared-${CLOUDFLARED_VERSION}.tgz`);
    await download(build, archive, onPercent);
    const unpacking = `${home}.unpacking`;
    fs.rmSync(unpacking, { recursive: true, force: true });
    fs.mkdirSync(unpacking, { recursive: true });
    await new Promise<void>((resolve, reject) =>
      execFile("tar", ["-xzf", archive, "-C", unpacking], (error) => (error ? reject(new Error(`Couldn't unpack ${archive}.`)) : resolve())),
    );
    fs.renameSync(unpacking, home);
    fs.rmSync(archive, { force: true });
  }
  if (process.platform !== "win32") fs.chmodSync(file, 0o755);
  return file;
}

export type TunnelEvent = { state: "connecting" } | { state: "up" } | { state: "down"; detail: string | null };

/** What cloudflared's log line means for the tunnel; null for lines that change nothing. */
export function readLogLine(line: string): TunnelEvent | null {
  // "Unregistered tunnel connection" ends in the words that mean up, so it is read first.
  if (/Unregistered tunnel connection|Connection terminated|Retrying connection|Serve tunnel error/i.test(line)) return { state: "connecting" };
  if (/\bRegistered tunnel connection/i.test(line)) return { state: "up" };
  return null;
}

export interface Tunnel {
  stop(): void;
}

/** Runs a tunnel; the token goes in TUNNEL_TOKEN, so it isn't on the command line other programs can read. */
export type StartTunnel = (file: string, token: string, onEvent: (event: TunnelEvent) => void) => Tunnel;

export const startTunnel: StartTunnel = (file, token, onEvent) => {
  const child = spawn(file, ["tunnel", "--no-autoupdate", "run"], {
    env: { ...process.env, TUNNEL_TOKEN: token },
    stdio: ["ignore", "pipe", "pipe"],
    windowsHide: true,
  });
  let stopped = false;
  // Why it stopped: its last error, or else its last words, such as "Provided Tunnel token is not valid."
  let lastError: string | null = null;
  let lastLine: string | null = null;
  let buffer = "";
  const read = (chunk: Buffer) => {
    buffer += chunk.toString("utf8");
    const lines = buffer.split(/\r?\n/);
    buffer = lines.pop() ?? "";
    for (const line of lines) {
      const text = line.replace(/^\S+\s+(ERR|INF|WRN|DBG)\s+/, "").trim();
      if (/\sERR\s/.test(line)) lastError = text;
      else if (text && !text.startsWith("See '")) lastLine = text;
      const event = readLogLine(line);
      if (event) onEvent(event);
    }
  };
  // cloudflared logs to stderr.
  child.stderr.on("data", read);
  child.stdout.on("data", read);
  child.on("error", (error) => {
    if (!stopped) onEvent({ state: "down", detail: error.message });
  });
  child.on("exit", (code) => {
    if (!stopped) onEvent({ state: "down", detail: lastError ?? lastLine ?? `cloudflared stopped (code ${code}).` });
  });
  const kill = () => child.kill();
  process.once("exit", kill);
  onEvent({ state: "connecting" });
  return {
    stop() {
      stopped = true;
      process.off("exit", kill);
      child.kill();
    },
  };
};
