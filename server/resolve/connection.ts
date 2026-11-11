// Linker-Edit's one connection to Resolve: Resolve's own MCP server (ResolveMCP), started as a child
// process and spoken to over stdio. Everything Linker-Edit does in Resolve is a Python script from
// server/resolve/scripts run through its run_script tool: the status check, history saves, previews,
// opening projects and the agents' Resolve tools. Calls go one at a time, in order.
import { spawn, type ChildProcess } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { createInterface } from "node:readline";
import { APP_ROOT } from "../config.ts";
import { MISSING_RESOLVE_MCP } from "./locate.ts";

/** Resolve's MCP server stops a script after this many seconds, its maximum. */
export const MAX_SCRIPT_SECONDS = 60;

/**
 * refused: the script said no, with a message for the user or agent. timeout: the script ran out of time.
 * error: the script crashed. unavailable: there is no answer from Resolve's MCP server at all.
 */
export type ResolveErrorKind = "refused" | "timeout" | "error" | "unavailable";

export class ResolveError extends Error {
  readonly kind: ResolveErrorKind;
  constructor(kind: ResolveErrorKind, message: string) {
    super(message);
    this.name = "ResolveError";
    this.kind = kind;
  }
}

export interface ResolveConnection {
  /** The ResolveMCP program, or null when this computer has none. */
  readonly command: string | null;
  /** Calls one of ResolveMCP's tools and returns its text. */
  callTool(name: string, args?: Record<string, unknown>, timeoutMs?: number): Promise<string>;
  /** Runs server/resolve/scripts/<name>.py with `ARGS` set to args, and returns its `result`. */
  runScript<T>(name: string, args?: unknown, timeoutSeconds?: number): Promise<T>;
  close(): void;
}

const SCRIPTS_DIR = path.join(APP_ROOT, "server", "resolve", "scripts");
const scriptCache = new Map<string, string>();

function scriptFile(name: string): string {
  let text = scriptCache.get(name);
  if (text === undefined) {
    if (!/^[a-z_]+$/.test(name)) throw new Error(`Bad script name ${name}.`);
    text = fs.readFileSync(path.join(SCRIPTS_DIR, `${name}.py`), "utf8");
    scriptCache.set(name, text);
  }
  return text;
}

/** The script ResolveMCP runs: ARGS, the shared helpers, and the script itself. */
export function scriptSource(name: string, args: unknown = {}): string {
  const parts = ["common", ...(name === "list_projects" || name === "open_project" ? ["projects_common"] : []), name];
  // A JSON string literal is a valid Python string literal too.
  return [`import json\nARGS = json.loads(${JSON.stringify(JSON.stringify(args ?? {}))})`, ...parts.map(scriptFile)].join("\n\n");
}

/** run_script's text: {"result": …} on success, {"error": traceback} on failure, plus any printed "output". */
export function parseScriptResult(text: string): unknown {
  let body: { result?: unknown; error?: unknown };
  try {
    body = JSON.parse(text) as typeof body;
  } catch {
    throw scriptError(text);
  }
  if (body && typeof body === "object" && "error" in body) throw scriptError(String(body.error));
  return body?.result ?? null;
}

function scriptError(text: string): ResolveError {
  const trimmed = text.trim();
  if (/^Script timed out/i.test(trimmed)) return new ResolveError("timeout", trimmed);
  // A traceback's last line names the exception; our own refusals carry a message for people.
  const last = trimmed.split(/\r?\n/).filter((line) => line.trim()).at(-1) ?? trimmed;
  const refused = /^Refused: (.*)$/.exec(last);
  if (refused) return new ResolveError("refused", refused[1]!);
  return new ResolveError("error", last || "Resolve's script failed without saying why.");
}

type Response = { id?: number; result?: { content?: { type: string; text?: string }[]; isError?: boolean; tools?: { name: string }[] }; error?: { message?: string } };

interface Pending {
  resolve(response: Response): void;
  reject(error: Error): void;
}

export interface ConnectionOptions {
  command: string | null;
  /** Extra arguments, for tests that run a stand-in server through Node. */
  args?: string[];
}

export function createResolveConnection({ command, args = [] }: ConnectionOptions): ResolveConnection {
  let child: ChildProcess | null = null;
  let ready: Promise<void> | null = null;
  let nextId = 0;
  const pending = new Map<number, Pending>();
  // One call at a time: Resolve does one thing at a time, and a stuck call is killed with its process.
  let queue: Promise<unknown> = Promise.resolve();

  function reset(reason: string) {
    const dead = child;
    child = null;
    ready = null;
    dead?.kill();
    for (const [id, waiter] of pending) {
      pending.delete(id);
      waiter.reject(new ResolveError("unavailable", reason));
    }
  }

  function request(method: string, params: unknown, timeoutMs: number): Promise<Response> {
    const proc = child;
    if (!proc?.stdin) return Promise.reject(new ResolveError("unavailable", "Resolve's MCP server isn't running."));
    const id = ++nextId;
    return new Promise<Response>((resolve, reject) => {
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new ResolveError("unavailable", "Resolve didn't answer in time. Is a dialog open in Resolve?"));
        // The call may still be running in Resolve; a fresh server process is the only clean way on.
        reset("Resolve didn't answer in time.");
      }, timeoutMs);
      pending.set(id, {
        resolve: (response) => {
          clearTimeout(timer);
          resolve(response);
        },
        reject: (error) => {
          clearTimeout(timer);
          reject(error);
        },
      });
      proc.stdin!.write(`${JSON.stringify({ jsonrpc: "2.0", id, method, params })}\n`);
    });
  }

  function start(): Promise<void> {
    if (!command) return Promise.reject(new ResolveError("unavailable", MISSING_RESOLVE_MCP));
    ready ??= (async () => {
      const spawned = spawn(command, args, { stdio: ["pipe", "pipe", "pipe"], windowsHide: true });
      child = spawned;
      let stderr = "";
      spawned.stderr?.on("data", (chunk: Buffer) => (stderr = (stderr + chunk.toString()).slice(-2000)));
      spawned.on("error", (error) => {
        if (child === spawned) reset(`Couldn't start Resolve's MCP server: ${error.message}`);
      });
      spawned.on("exit", (code) => {
        const detail = stderr.trim().split("\n").at(-1);
        if (child === spawned) reset(`Resolve's MCP server stopped (exit ${code})${detail ? `: ${detail}` : "."}`);
      });
      createInterface({ input: spawned.stdout! }).on("line", (line) => {
        let message: Response;
        try {
          message = JSON.parse(line) as Response;
        } catch {
          return;
        }
        if (typeof message.id !== "number") return;
        const waiter = pending.get(message.id);
        pending.delete(message.id);
        waiter?.resolve(message);
      });
      await request("initialize", { protocolVersion: "2025-06-18", capabilities: {}, clientInfo: { name: "linker-edit", version: "1" } }, 30_000);
      spawned.stdin!.write(`${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`);
      const listed = await request("tools/list", {}, 30_000);
      if (!listed.result?.tools?.some((tool) => tool.name === "run_script")) {
        throw new ResolveError("unavailable", "This Resolve's MCP server has no run_script tool. Update DaVinci Resolve Studio.");
      }
    })();
    // A failed start is retried on the next call rather than remembered.
    ready.catch(() => reset("Resolve's MCP server didn't start."));
    return ready;
  }

  async function callNow(name: string, args: Record<string, unknown>, timeoutMs: number): Promise<string> {
    await start();
    const response = await request("tools/call", { name, arguments: args }, timeoutMs);
    if (response.error) throw new ResolveError("error", response.error.message ?? `Resolve's ${name} failed.`);
    const text = (response.result?.content ?? []).map((part) => (part.type === "text" ? (part.text ?? "") : `[${part.type}]`)).join("\n");
    if (response.result?.isError) throw new ResolveError("error", text || `Resolve's ${name} failed.`);
    return text;
  }

  function callTool(name: string, args: Record<string, unknown> = {}, timeoutMs = 30_000): Promise<string> {
    const call = queue.then(() => callNow(name, args, timeoutMs));
    queue = call.catch(() => {});
    return call;
  }

  return {
    command,
    callTool,
    async runScript<T>(name: string, args?: unknown, timeoutSeconds = 30): Promise<T> {
      const seconds = Math.min(Math.max(1, Math.round(timeoutSeconds)), MAX_SCRIPT_SECONDS);
      const text = await callTool("run_script", { script: scriptSource(name, args), timeout: seconds }, (seconds + 20) * 1000);
      return parseScriptResult(text) as T;
    },
    close: () => reset("Linker-Edit closed the connection."),
  };
}
