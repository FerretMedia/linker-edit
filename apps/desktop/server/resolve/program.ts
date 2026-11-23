import { execFile, spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { ResolveError } from "./connection.ts";

export interface ResolveProcesses {
  /** Resolve's process id, or null when it isn't running. */
  resolve: number | null;
  /** fuscript, which answers scripts. Resolve starts it late in its boot. */
  scriptServer: boolean;
}

export function parseProcesses(output: string, platform: NodeJS.Platform): ResolveProcesses {
  const rows = platform === "win32" ? /^"(?<program>[^"]+)","(?<pid>\d+)",/gm : /^ *(?<pid>\d+) +(?<program>.*\S)/gm;
  const pids = new Map<string, number>();
  for (const { groups } of output.matchAll(rows)) {
    pids.set(path.posix.basename(groups!.program!).toLowerCase().replace(/\.exe$/, ""), Number(groups!.pid));
  }
  if (!pids.size) throw new Error("no processes listed");
  return { resolve: pids.get("resolve") ?? null, scriptServer: pids.has("fuscript") };
}

const unavailable = (what: string, error: unknown) =>
  new ResolveError("unavailable", `${what} (${error instanceof Error ? error.message : String(error)}). Start Resolve yourself, then try again.`);

export function resolveProcesses(): Promise<ResolveProcesses> {
  // -ww: macOS's ps can cut lines, and Resolve's programs have long paths there.
  const [command, args] = process.platform === "win32" ? ["tasklist.exe", ["/nh", "/fo", "csv"]] : ["ps", ["-A", "-ww", "-o", "pid=,comm="]];
  return new Promise((resolve, reject) => {
    execFile(command, args, { windowsHide: true, timeout: 10_000, maxBuffer: 8 * 1024 * 1024 }, (error, stdout) => {
      try {
        if (error) throw error;
        resolve(parseProcesses(stdout, process.platform));
      } catch (problem) {
        reject(unavailable("Linker-Edit can't see whether Resolve is running", problem));
      }
    });
  });
}

export function isAlive(pid: number): boolean {
  try {
    return process.kill(pid, 0);
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

export function launchCommand(resolveMcp: string, platform: NodeJS.Platform): { program: string; command: string; args: string[] } {
  if (platform === "win32") {
    const program = path.win32.join(path.win32.dirname(resolveMcp), "Resolve.exe");
    // Through a cmd that ends at once: quitting the app ends every process under the server with taskkill /T,
    // and a Resolve with unsaved work must not be one of them.
    return { program, command: "cmd.exe", args: ["/d", "/s", "/c", `"start "" "${program}""`] };
  }
  if (platform === "darwin") {
    // ResolveMCP is in the app's Contents/Applications.
    const program = path.posix.join(resolveMcp, "../../..");
    return { program, command: "open", args: [program] };
  }
  const program = path.posix.join(path.posix.dirname(resolveMcp), "resolve");
  return { program, command: program, args: [] };
}

/** Without what only the server should see: Resolve's workflow integrations are Electron apps themselves. */
export function launchEnv(env: NodeJS.ProcessEnv): NodeJS.ProcessEnv {
  return Object.fromEntries(Object.entries(env).filter(([name]) => !/^(ELECTRON_RUN_AS_NODE$|LINKER_EDIT_)/i.test(name)));
}

export async function launchResolve(resolveMcp: string): Promise<void> {
  const { program, command, args } = launchCommand(resolveMcp, process.platform);
  if (!fs.existsSync(program)) {
    throw new ResolveError("unavailable", `Linker-Edit can't start Resolve: ${program} is missing. Start Resolve yourself, then try again.`);
  }
  try {
    await new Promise<void>((resolve, reject) => {
      const child = spawn(command, args, {
        stdio: "ignore",
        windowsHide: true,
        windowsVerbatimArguments: true,
        // Its own process group on Linux: quitting the app ends the server's group.
        detached: process.platform !== "win32",
        // Not the server's folder, which Resolve would keep in use.
        cwd: path.dirname(program),
        env: launchEnv(process.env),
      });
      child.on("error", reject);
      if (command === program) child.on("spawn", resolve);
      else child.on("exit", (code) => (code === 0 ? resolve() : reject(new Error(`${command} stopped with exit code ${code}`))));
      child.unref();
    });
  } catch (error) {
    throw unavailable("Linker-Edit couldn't start Resolve", error);
  }
}
