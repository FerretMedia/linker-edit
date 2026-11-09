import { randomBytes } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

export const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Config, workspace and the davinci-resolve-mcp install live here, outside the app folder npx replaces. */
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
  /** Folder of a davinci-resolve-mcp checkout with its .venv. */
  resolveMcpDir?: string;
  /** Python that runs the MCP server. Defaults to the checkout's .venv. */
  python?: string;
  /** Extra or replacement environment variables for the MCP server. */
  env?: Record<string, string>;
  /** Tokens at which Claude compacts the chat by itself, 100000 to 1000000. Claude Code decides when unset. */
  autoCompactWindow?: number;
  /** Where transcripts of media files are kept. */
  transcriptsDir?: string;
  /** Where the speech engine and its models are downloaded. */
  toolsDir?: string;
  /** Your own whisper-cli, instead of the one Linker-Edit downloads (Windows) or finds on PATH. */
  whisperCli?: string;
}

export interface ResolveMcpConfig {
  python: string;
  serverScript: string;
  env: Record<string, string>;
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

function defaultScriptingEnv(platform: NodeJS.Platform): Record<string, string> {
  if (platform === "win32") {
    const api = path.join(
      process.env.PROGRAMDATA ?? "C:\\ProgramData",
      "Blackmagic Design",
      "DaVinci Resolve",
      "Support",
      "Developer",
      "Scripting",
    );
    return {
      RESOLVE_SCRIPT_API: api,
      RESOLVE_SCRIPT_LIB: path.join(
        process.env.PROGRAMFILES ?? "C:\\Program Files",
        "Blackmagic Design",
        "DaVinci Resolve",
        "fusionscript.dll",
      ),
      PYTHONPATH: path.join(api, "Modules"),
    };
  }
  if (platform === "darwin") {
    const api = "/Library/Application Support/Blackmagic Design/DaVinci Resolve/Developer/Scripting";
    return {
      RESOLVE_SCRIPT_API: api,
      RESOLVE_SCRIPT_LIB:
        "/Applications/DaVinci Resolve/DaVinci Resolve.app/Contents/Libraries/Fusion/fusionscript.so",
      PYTHONPATH: path.join(api, "Modules"),
    };
  }
  const api = "/opt/resolve/Developer/Scripting";
  return {
    RESOLVE_SCRIPT_API: api,
    RESOLVE_SCRIPT_LIB: "/opt/resolve/libs/Fusion/fusionscript.so",
    PYTHONPATH: path.join(api, "Modules"),
  };
}

// fusionscript.dll needs the base interpreter's home on Windows, which a venv hides.
function venvBaseHome(python: string): string | undefined {
  const cfg = path.join(path.dirname(path.dirname(python)), "pyvenv.cfg");
  if (!fs.existsSync(cfg)) return undefined;
  return /^home\s*=\s*(.+)$/m.exec(fs.readFileSync(cfg, "utf8"))?.[1]?.trim();
}

/** LINKER_EDIT_CONFIG points at another config file, so tests can run on their own port and workspace. */
export function loadConfig(
  file = process.env.LINKER_EDIT_CONFIG ?? path.join(HOME_DIR, "linker-edit.config.json"),
  platform: NodeJS.Platform = process.platform,
): AppConfig {
  const user = readUserConfig(file);
  const window = user.autoCompactWindow;
  if (window !== undefined && !(Number.isInteger(window) && window >= 100_000 && window <= 1_000_000)) {
    throw new Error(`autoCompactWindow in ${file} must be a whole number from 100000 to 1000000.`);
  }
  const mcpDir = user.resolveMcpDir ?? path.join(HOME_DIR, "davinci-resolve-mcp");
  const python =
    user.python ??
    (platform === "win32"
      ? path.join(mcpDir, ".venv", "Scripts", "python.exe")
      : path.join(mcpDir, ".venv", "bin", "python"));

  const env = defaultScriptingEnv(platform);
  const home = platform === "win32" ? venvBaseHome(python) : undefined;
  if (home) env.PYTHONHOME = home;

  return {
    port: user.port ?? 4317,
    projectsDir: user.projectsDir ?? path.join(HOME_DIR, "projects"),
    workspaceDir: user.workspaceDir ?? path.join(HOME_DIR, "workspace"),
    previewDir: user.previewDir ?? path.join(HOME_DIR, "previews"),
    transcriptsDir: user.transcriptsDir ?? path.join(HOME_DIR, "transcripts"),
    toolsDir: user.toolsDir ?? path.join(HOME_DIR, "tools"),
    ...(user.whisperCli ? { whisperCli: user.whisperCli } : {}),
    apiToken: randomBytes(24).toString("hex"),
    resolveMcp: {
      python,
      serverScript: path.join(mcpDir, "src", "server.py"),
      env: { ...env, ...user.env },
    },
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
