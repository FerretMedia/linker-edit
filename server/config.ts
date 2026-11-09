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
  workspaceDir?: string;
  /** Where preview renders go. Kept out of the workspace so they never show in its Git status. */
  previewDir?: string;
  /** Folder of a davinci-resolve-mcp checkout with its .venv. */
  resolveMcpDir?: string;
  /** Python that runs the MCP server. Defaults to the checkout's .venv. */
  python?: string;
  /** Extra or replacement environment variables for the MCP server. */
  env?: Record<string, string>;
}

export interface ResolveMcpConfig {
  python: string;
  serverScript: string;
  env: Record<string, string>;
}

export interface AppConfig {
  port: number;
  workspaceDir: string;
  previewDir: string;
  resolveMcp: ResolveMcpConfig;
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
    workspaceDir: user.workspaceDir ?? path.join(HOME_DIR, "workspace"),
    previewDir: user.previewDir ?? path.join(HOME_DIR, "previews"),
    resolveMcp: {
      python,
      serverScript: path.join(mcpDir, "src", "server.py"),
      env: { ...env, ...user.env },
    },
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
