// Where DaVinci Resolve Studio 21.1.2 and newer install their own MCP server. It's Blackmagic Design's
// proprietary program: Linker-Edit starts the user's installed copy and never ships one.
import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";

const MAC_PATHS = [
  "/Applications/DaVinci Resolve/DaVinci Resolve.app/Contents/Applications/ResolveMCP",
  "/Applications/DaVinci Resolve Studio.app/Contents/Applications/ResolveMCP",
];
const LINUX_PATH = "/opt/resolve/bin/ResolveMCP";

/** Resolve's install folder from the .drp file association, for installs outside Program Files. */
function windowsInstallDir(): string | null {
  try {
    const out = execFileSync("reg.exe", ["query", "HKCR\\ResolveDrpFile\\shell\\open\\command", "/ve"], {
      encoding: "utf8",
      timeout: 10_000,
      windowsHide: true,
    });
    const exe = /REG_SZ\s+"([^"]+)"/.exec(out)?.[1];
    return exe ? path.dirname(exe) : null;
  } catch {
    return null;
  }
}

export interface LocateOptions {
  platform?: NodeJS.Platform;
  env?: NodeJS.ProcessEnv;
  exists?: (file: string) => boolean;
  installDir?: () => string | null;
}

/** The ResolveMCP program, or null when this Resolve doesn't have one (not installed, or older than 21.1.2). */
export function findResolveMcp({
  platform = process.platform,
  env = process.env,
  exists = fs.existsSync,
  installDir = windowsInstallDir,
}: LocateOptions = {}): string | null {
  if (platform === "win32") {
    const programFiles = env.ProgramFiles ?? "C:\\Program Files";
    const usual = path.win32.join(programFiles, "Blackmagic Design", "DaVinci Resolve", "ResolveMCP.exe");
    if (exists(usual)) return usual;
    const dir = installDir();
    const registered = dir ? path.win32.join(dir, "ResolveMCP.exe") : null;
    return registered && exists(registered) ? registered : null;
  }
  if (platform === "darwin") return MAC_PATHS.find((file) => exists(file)) ?? null;
  return exists(LINUX_PATH) ? LINUX_PATH : null;
}

export const MISSING_RESOLVE_MCP =
  "Linker-Edit drives Resolve through the MCP server that comes with DaVinci Resolve Studio 21.1.2 and newer, and can't find it. Install or update DaVinci Resolve Studio, then refresh.";
