import { randomBytes } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";
import type { GoogleClient } from "./drive/auth.ts";
import { findResolveMcp } from "./resolve/locate.ts";

export const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Config and workspace live here, outside the app folder npx replaces. */
export const HOME_DIR = path.join(os.homedir(), "LinkerEdit");

/** Linker's own sign-in (production Clerk). */
export const LINKER_AUTH_SERVER = "https://clerk.linker.cool";

/**
 * Linker-Edit's OAuth application in Linker's production Clerk: a public client (PKCE, no secret) whose redirect
 * addresses are http://127.0.0.1:4317 and http://localhost:4317 /api/linker/callback. Client ids are public. Clerk
 * can't read the Client ID Metadata Document Linker publishes (`invalid_client`), so this is the default.
 */
export const LINKER_CLIENT_ID = "okxg8tSdwQEu1rbv";

/**
 * Linker-Edit's OAuth client in Google Cloud, of type Desktop app, for Google Drive; null until it's made. Google says
 * a desktop client's secret isn't confidential. LINKER_EDIT_GOOGLE_CLIENT_ID and _SECRET in the environment win.
 */
export const GOOGLE_CLIENT: GoogleClient | null = null;

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
  /** Your own whisper-cli, instead of the one Linker-Edit finds on PATH or downloads. */
  whisperCli?: string;
  /** The HTTPS port remote access uses on your tailnet. */
  tailscalePort?: number;
  /** Linker's sign-in (Clerk), for testing against staging. */
  linkerAuthServer?: string;
  /** Linker's dashboard and API, for testing against staging. */
  linkerAppUrl?: string;
  /** An OAuth client id for signing in with Linker, instead of Linker-Edit's own (LINKER_CLIENT_ID). */
  linkerClientId?: string;
  /** Your own cloudflared for Linker Connect, instead of one on PATH or the one Linker-Edit downloads. */
  cloudflared?: string;
  /** A stand-in for Google's sign-in and Drive API, for testing. */
  googleUrl?: string;
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
  /** This configuration's own file, which the user writes by hand; it may not exist. */
  configFile: string;
  /** The user's choices kept across restarts, such as supervised mode. */
  settingsFile: string;
  /** The universal library's folders, and the tags, stars and notes on library files. */
  libraryFile: string;
  /** What's in each library folder, so a rescan only measures changed files. */
  libraryCacheDir: string;
  /** Signing in with a Linker account; file is where the sign-in is kept. */
  linker: { authServer: string; appUrl: string; clientId?: string; file: string };
  /** Signing in to Google Drive; no client means this build can't. */
  google: { client: GoogleClient | null; file: string; url?: string };
  /** Linker Connect: this computer's link and tunnel token, the devices signed in through it, and cloudflared. */
  connect: { file: string; sessionsFile: string; cloudflared?: string };
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
  env: Record<string, string | undefined> = process.env,
): AppConfig {
  const user = readUserConfig(file);
  const window = user.autoCompactWindow;
  if (window !== undefined && !(Number.isInteger(window) && window >= 100_000 && window <= 1_000_000)) {
    throw new Error(`autoCompactWindow in ${file} must be a whole number from 100000 to 1000000.`);
  }
  const port = user.port ?? 4317;
  if (!(Number.isInteger(port) && port > 0 && port < 65536)) {
    throw new Error(`port in ${file} must be a port number.`);
  }
  const authServer = user.linkerAuthServer ?? LINKER_AUTH_SERVER;
  // Linker-Edit's own client exists only in production Clerk; against another Clerk (staging), the published
  // document or registering itself still applies unless linkerClientId says otherwise.
  const clientId = user.linkerClientId ?? (authServer.replace(/\/$/, "") === LINKER_AUTH_SERVER ? LINKER_CLIENT_ID : undefined);
  const googleId = env.LINKER_EDIT_GOOGLE_CLIENT_ID;
  const googleSecret = env.LINKER_EDIT_GOOGLE_CLIENT_SECRET;
  const tailscalePort = user.tailscalePort ?? 8443;
  if (!(Number.isInteger(tailscalePort) && tailscalePort > 0 && tailscalePort < 65536)) {
    throw new Error(`tailscalePort in ${file} must be a port number.`);
  }
  return {
    port,
    projectsDir: user.projectsDir ?? path.join(HOME_DIR, "projects"),
    workspaceDir: user.workspaceDir ?? path.join(HOME_DIR, "workspace"),
    previewDir: user.previewDir ?? path.join(HOME_DIR, "previews"),
    transcriptsDir: user.transcriptsDir ?? path.join(HOME_DIR, "transcripts"),
    toolsDir: user.toolsDir ?? path.join(HOME_DIR, "tools"),
    ...(user.whisperCli ? { whisperCli: user.whisperCli } : {}),
    tailscalePort,
    configFile: file,
    settingsFile: path.join(path.dirname(file), "settings.json"),
    libraryFile: path.join(path.dirname(file), "library.json"),
    libraryCacheDir: path.join(path.dirname(file), "cache", "library"),
    linker: {
      authServer,
      appUrl: user.linkerAppUrl ?? "https://app.linker.cool",
      ...(clientId ? { clientId } : {}),
      file: path.join(path.dirname(file), "linker-account.json"),
    },
    google: {
      client: googleId && googleSecret ? { id: googleId, secret: googleSecret } : GOOGLE_CLIENT,
      file: path.join(path.dirname(file), "google-account.json"),
      ...(user.googleUrl ? { url: user.googleUrl } : {}),
    },
    connect: {
      file: path.join(path.dirname(file), "connect.json"),
      sessionsFile: path.join(path.dirname(file), "connect-sessions.json"),
      ...(user.cloudflared ? { cloudflared: user.cloudflared } : {}),
    },
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
