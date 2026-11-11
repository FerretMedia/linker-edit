import { randomBytes } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import { findResolveMcp } from "./resolve/locate.ts";

export const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Config and workspace live here, outside the app folder npx replaces. */
export const HOME_DIR = path.join(os.homedir(), "LinkerEdit");

/** Optional overrides, read from linker-edit.config.json in HOME_DIR. */
interface UserConfig {
  port?: number;
  /** Where new projects get their folders, and the project list lives. */
  projectsDir?: string;
  /** The working folder from before projects; it becomes the first project. */
  workspaceDir?: string;
  /** Where preview renders go. Kept out of the workspace so they never show in its Git status. */
  previewDir?: string;
  /** Resolve's own MCP server (ResolveMCP), when Resolve is installed somewhere Linker-Edit doesn't look. */
  resolveMcp?: string;
  /** Tokens at which Claude compacts the chat by itself, 100000 to 1000000. Claude Code decides when unset. */
  autoCompactWindow?: number;
  /** Where transcripts of media files are kept. */
  transcriptsDir?: string;
  /** Where the speech engine and its models are downloaded. */
  toolsDir?: string;
  /** Your own whisper-cli, instead of the one Linker-Edit downloads (Windows) or finds on PATH. */
  whisperCli?: string;
  /** The HTTPS port remote access uses on your tailnet. */
  tailscalePort?: number;
}

export interface ResolveMcpConfig {
  /** Resolve's own MCP server program; null when this computer has none. */
  command: string | null;
}

export interface AppConfig {
  port: number;
  projectsDir: string;
  /** The agent's working folder. Each project has its own; this one becomes the first project. */
  workspaceDir: string;
  previewDir: string;
  transcriptsDir: string;
  toolsDir: string;
  whisperCli?: string;
  tailscalePort: number;
  /** Lets the agents' MCP server call this server's API; web pages and other programs can't. */
  apiToken: string;
  resolveMcp: ResolveMcpConfig;
  autoCompactWindow?: number;
}

function readUserConfig(file: string): UserConfig {
  if (!fs.existsSync(file)) return {};
  try {
    return JSON.parse(fs.readFileSync(file, "utf8")) as UserConfig;
  } catch (error) {
    throw new Error(`Can't read ${file}: ${error instanceof Error ? error.message : String(error)}`);
  }
}

/** LINKER_EDIT_CONFIG points at another config file, so tests can run on their own port and workspace. */
export function loadConfig(
  file = process.env.LINKER_EDIT_CONFIG ?? path.join(HOME_DIR, "linker-edit.config.json"),
  findMcp: () => string | null = findResolveMcp,
): AppConfig {
  const user = readUserConfig(file);
  const window = user.autoCompactWindow;
  if (window !== undefined && !(Number.isInteger(window) && window >= 100_000 && window <= 1_000_000)) {
    throw new Error(`autoCompactWindow in ${file} must be a whole number from 100000 to 1000000.`);
  }
  const tailscalePort = user.tailscalePort ?? 8443;
  if (!(Number.isInteger(tailscalePort) && tailscalePort > 0 && tailscalePort < 65536)) {
    throw new Error(`tailscalePort in ${file} must be a port number.`);
  }
  return {
    port: user.port ?? 4317,
    projectsDir: user.projectsDir ?? path.join(HOME_DIR, "projects"),
    workspaceDir: user.workspaceDir ?? path.join(HOME_DIR, "workspace"),
    previewDir: user.previewDir ?? path.join(HOME_DIR, "previews"),
    transcriptsDir: user.transcriptsDir ?? path.join(HOME_DIR, "transcripts"),
    toolsDir: user.toolsDir ?? path.join(HOME_DIR, "tools"),
    ...(user.whisperCli ? { whisperCli: user.whisperCli } : {}),
    tailscalePort,
    apiToken: randomBytes(24).toString("hex"),
    resolveMcp: { command: user.resolveMcp ?? findMcp() },
    ...(window === undefined ? {} : { autoCompactWindow: window }),
  };
}

export function loadConfigOrExit(): AppConfig {
  try {
    return loadConfig();
  } catch (error) {
    console.error(error instanceof Error ? error.message : error);
    process.exit(1);
  }
}
