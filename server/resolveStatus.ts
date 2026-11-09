import { execFile } from "node:child_process";
import type { ResolveStatus } from "../shared/protocol.ts";
import type { ResolveMcpConfig } from "./config.ts";
import { isResolveMcpInstalled } from "./setup.ts";

const PROBE = `
import json, sys
try:
    import DaVinciResolveScript as d
except Exception as e:
    print(json.dumps({"state": "no_module", "detail": str(e)})); sys.exit()
r = d.scriptapp("Resolve")
if not r:
    print(json.dumps({"state": "unreachable"})); sys.exit()
out = {"state": "connected", "product": r.GetProductName(), "version": r.GetVersionString(), "project": None, "timeline": None, "page": r.GetCurrentPage()}
p = r.GetProjectManager().GetCurrentProject()
if p:
    out["project"] = p.GetName()
    t = p.GetCurrentTimeline()
    out["timeline"] = t.GetName() if t else None
print(json.dumps(out))
`;

const BLOCKED_HINT =
  "Resolve answers but refuses edits while the Project Manager or a dialog, such as an update prompt, is open. Open a project and close the dialog in Resolve, then refresh.";

const UNREACHABLE_HINT =
  "Resolve is running but refuses scripting. Use DaVinci Resolve Studio, set Preferences → System → General → External scripting using: Local, then restart Resolve.";

function run(file: string, args: string[], env?: Record<string, string>): Promise<string> {
  return new Promise((resolve) => {
    execFile(
      file,
      args,
      { timeout: 15_000, windowsHide: true, env: env ? { ...process.env, ...env } : process.env },
      (_error, stdout) => resolve(stdout),
    );
  });
}

async function isResolveRunning(): Promise<boolean> {
  if (process.platform === "win32") {
    const out = await run("tasklist", ["/FI", "IMAGENAME eq Resolve.exe", "/NH"]);
    return out.toLowerCase().includes("resolve.exe");
  }
  const out = await run("pgrep", ["-x", process.platform === "darwin" ? "Resolve" : "resolve"]);
  return out.trim().length > 0;
}

export async function checkResolve(
  mcp: ResolveMcpConfig,
  isRunning: () => Promise<boolean> = isResolveRunning,
): Promise<ResolveStatus> {
  if (!isResolveMcpInstalled(mcp)) {
    return {
      state: "setup_needed",
      hint: `davinci-resolve-mcp not found (expected ${mcp.serverScript} and its .venv). Run \`npx linker-edit@latest setup\`, then refresh.`,
    };
  }
  if (!(await isRunning())) return { state: "not_running" };
  return parseProbeOutput(await run(mcp.python, ["-c", PROBE], mcp.env));
}

/** The probe prints one JSON line last; anything else means it crashed or timed out. */
export function parseProbeOutput(stdout: string): ResolveStatus {
  const out = stdout.trim().split("\n").pop() ?? "";
  try {
    const result = JSON.parse(out) as { state: string; detail?: string } & Record<string, unknown>;
    if (result.state === "connected") {
      const { page, ...status } = result;
      // No page means Resolve is on the Project Manager or behind a modal dialog.
      if (page === null) return { state: "blocked", hint: BLOCKED_HINT };
      return status as unknown as ResolveStatus;
    }
    if (result.state === "no_module") {
      return { state: "setup_needed", hint: `Resolve scripting module failed to load: ${result.detail}` };
    }
  } catch {
    // Probe crashed or timed out; treat it like a refused connection.
  }
  return { state: "unreachable", hint: UNREACHABLE_HINT };
}
