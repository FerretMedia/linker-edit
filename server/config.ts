import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { fileURLToPath } from "node:url";

export const APP_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

/** Optional overrides, read from linker-edit.config.json in the app folder. */
interface UserConfig {
  port?: number;
  workspaceDir?: string;
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
  file = process.env.LINKER_EDIT_CONFIG ?? path.join(APP_ROOT, "linker-edit.config.json"),
  platform: NodeJS.Platform = process.platform,
): AppConfig {
  const user = readUserConfig(file);
  const mcpDir = user.resolveMcpDir ?? path.resolve(APP_ROOT, "..", "davinci-resolve-mcp");
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
    workspaceDir: user.workspaceDir ?? path.join(os.homedir(), "LinkerEdit", "workspace"),
    resolveMcp: {
      python,
      serverScript: path.join(mcpDir, "src", "server.py"),
      env: { ...env, ...user.env },
    },
  };
}
