import { execFile } from "node:child_process";
import * as fs from "node:fs";
import type { IncomingHttpHeaders } from "node:http";
import * as path from "node:path";
import type { RemoteAccess } from "../shared/protocol.ts";

export interface TailscaleResult {
  code: number;
  stdout: string;
  stderr: string;
  timedOut: boolean;
}

/** Runs the tailscale CLI; null when it isn't installed. */
export type RunTailscale = (args: string[], timeoutMs: number) => Promise<TailscaleResult | null>;

/** Who may come in over Tailscale: the tailnet address this server is served at, and the account that owns this computer. */
export interface RemoteGate {
  /** host[:port] as browsers send it, such as studio.tail1234.ts.net:8443. */
  host: string;
  owner: string;
}

const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);
const DOWNLOAD = "https://tailscale.com/download";
const DNS_SETTINGS = "https://login.tailscale.com/admin/dns";

function tailscaleCommand(): string {
  const installed =
    process.platform === "win32"
      ? path.join(process.env.PROGRAMFILES ?? "C:\\Program Files", "Tailscale", "tailscale.exe")
      : process.platform === "darwin"
        ? "/Applications/Tailscale.app/Contents/MacOS/Tailscale"
        : null;
  return installed && fs.existsSync(installed) ? installed : "tailscale";
}

export const runTailscale: RunTailscale = (args, timeoutMs) =>
  new Promise((resolve) => {
    execFile(tailscaleCommand(), args, { timeout: timeoutMs, windowsHide: true, maxBuffer: 8 * 1024 * 1024 }, (error, stdout, stderr) => {
      if ((error as NodeJS.ErrnoException | null)?.code === "ENOENT") {
        resolve(null);
        return;
      }
      const code = error ? (typeof error.code === "number" ? error.code : 1) : 0;
      resolve({ code, stdout, stderr, timedOut: error?.killed === true });
    });
  });

function hostOf(value: string | undefined): string | null {
  if (!value) return null;
  try {
    return new URL(`https://${value}`).host.toLowerCase();
  } catch {
    return null;
  }
}

/**
 * Where a request comes from: this computer, the owner's devices through Tailscale Serve, or neither.
 * Tailscale Serve proxies from 127.0.0.1, keeps the tailnet Host, adds X-Forwarded-For and sets the
 * Tailscale-User-Login of the device's account after dropping any the client sent.
 */
export function requestSource(headers: IncomingHttpHeaders, gate: RemoteGate | null): "local" | "remote" | null {
  const host = hostOf(headers.host);
  const hostname = host ? new URL(`https://${host}`).hostname : null;
  const proxied = headers["x-forwarded-for"] !== undefined || headers["tailscale-user-login"] !== undefined;
  if (hostname && LOOPBACK_HOSTS.has(hostname) && !proxied) return "local";
  if (!gate || !proxied) return null;
  // Should Serve ever stop keeping the Host, the forwarded one still names the tailnet address.
  const publicHost = hostname && LOOPBACK_HOSTS.has(hostname) ? hostOf(String(headers["x-forwarded-host"] ?? "")) : host;
  const login = headers["tailscale-user-login"];
  const owner = typeof login === "string" && login.toLowerCase() === gate.owner.toLowerCase();
  return publicHost === hostOf(gate.host) && owner ? "remote" : null;
}

export function remoteUrl(host: string, servePort: number): string {
  return servePort === 443 ? `https://${host}` : `https://${host}:${servePort}`;
}

type Tailnet = { ok: true; host: string; owner: string; httpsCerts: boolean } | { ok: false; hint: string };

interface StatusJson {
  BackendState?: string;
  Self?: { DNSName?: string; UserID?: number; CapMap?: Record<string, unknown> | null };
  User?: Record<string, { LoginName?: string }>;
}

export function parseStatus(result: TailscaleResult | null): Tailnet {
  if (!result) return { ok: false, hint: `Install Tailscale on this computer and the other one, and log in to the same account: ${DOWNLOAD}` };
  let status: StatusJson;
  try {
    status = JSON.parse(result.stdout) as StatusJson;
  } catch {
    return { ok: false, hint: "Start Tailscale on this computer." };
  }
  switch (status.BackendState) {
    case "Running":
      break;
    case "NeedsLogin":
    case "NoState":
      return { ok: false, hint: "Log in to Tailscale on this computer." };
    case "NeedsMachineAuth":
      return { ok: false, hint: "This computer is waiting for approval in the Tailscale admin console." };
    case "Stopped":
      return { ok: false, hint: "Tailscale is turned off on this computer. Turn it on." };
    default:
      return { ok: false, hint: "Tailscale is starting…" };
  }
  const host = status.Self?.DNSName?.replace(/\.$/, "").toLowerCase();
  if (!host) return { ok: false, hint: `Turn on MagicDNS in the Tailscale admin console: ${DNS_SETTINGS}` };
  const owner = status.User?.[String(status.Self?.UserID)]?.LoginName;
  if (!owner) return { ok: false, hint: "Tailscale doesn't say which account this computer belongs to. Log in to Tailscale with your own account." };
  // Older CLIs report no CapMap; serve will say so itself if certificates are off.
  const capMap = status.Self?.CapMap;
  return { ok: true, host, owner, httpsCerts: !capMap || "https" in capMap };
}

interface ServeJson {
  TCP?: Record<string, unknown>;
  Web?: Record<string, { Handlers?: Record<string, { Proxy?: string }> }>;
}

const normalizeTarget = (target: string) => (/^[a-z]+:\/\//i.test(target) ? target : `http://${target}`).replace(/\/$/, "");

/** What Tailscale Serve sends the HTTPS port to: "this" server, "nothing", or something else. */
export function servedOn(serveJson: string, host: string, servePort: number, target: string): "this" | "nothing" | "other" {
  let serve: ServeJson;
  try {
    serve = (JSON.parse(serveJson || "{}") ?? {}) as ServeJson;
  } catch {
    return "nothing";
  }
  const proxy = serve.Web?.[`${host}:${servePort}`]?.Handlers?.["/"]?.Proxy;
  if (proxy && normalizeTarget(proxy) === normalizeTarget(target)) return "this";
  return proxy || serve.TCP?.[String(servePort)] ? "other" : "nothing";
}

function lastLines(text: string): string {
  return text.trim().split(/\r?\n/).slice(-4).join(" ").trim();
}

export interface RemoteAccessOptions {
  /** This server's port on 127.0.0.1. */
  port: number;
  /** The HTTPS port on the tailnet. */
  servePort: number;
  run?: RunTailscale;
}

export type RemoteAccessService = ReturnType<typeof createRemoteAccess>;

/** Serves this app to the owner's other devices with Tailscale Serve. Tailscale keeps that setting across restarts. */
export function createRemoteAccess(options: RemoteAccessOptions) {
  const run = options.run ?? runTailscale;
  const target = `http://127.0.0.1:${options.port}`;
  let status: RemoteAccess = { state: "checking" };
  let gate: RemoteGate | null = null;
  let switching = false;
  // Never take over a port another Serve setting uses.
  let portTaken = false;
  const listeners = new Set<() => void>();

  function set(next: RemoteAccess) {
    status = next;
    for (const listener of listeners) listener();
  }

  async function read(): Promise<{ tailnet: Tailnet; served: "this" | "nothing" | "other" }> {
    const tailnet = parseStatus(await run(["status", "--json"], 10_000));
    if (!tailnet.ok) return { tailnet, served: "nothing" };
    const serve = await run(["serve", "status", "--json"], 10_000);
    return { tailnet, served: servedOn(serve?.stdout ?? "", tailnet.host, options.servePort, target) };
  }

  /** Reads Tailscale's state; problem is kept on screen when remote access is still off. */
  async function refresh(problem: string | null = null): Promise<void> {
    const { tailnet, served } = await read();
    portTaken = served === "other";
    if (!tailnet.ok) {
      gate = null;
      set({ state: "unavailable", hint: tailnet.hint });
      return;
    }
    const url = remoteUrl(tailnet.host, options.servePort);
    if (served === "this") {
      gate = { host: new URL(url).host, owner: tailnet.owner };
      set({ state: "on", url, owner: tailnet.owner });
      return;
    }
    gate = null;
    const conflict = portTaken
      ? `Tailscale already serves something else on port ${options.servePort}. Set "tailscalePort" in linker-edit.config.json to another port.`
      : null;
    const certs = tailnet.httpsCerts ? null : `Turn on HTTPS certificates in the Tailscale admin console: ${DNS_SETTINGS}`;
    set({ state: "off", url, problem: conflict ?? problem ?? certs });
  }

  async function setEnabled(enabled: boolean): Promise<void> {
    if (switching || status.state === "checking" || status.state === "unavailable") return;
    if (enabled ? status.state !== "off" || portTaken : status.state !== "on") return;
    switching = true;
    set({ state: "switching", url: status.url });
    try {
      const result = await run(
        enabled ? ["serve", "--bg", `--https=${options.servePort}`, target] : ["serve", `--https=${options.servePort}`, "off"],
        30_000,
      );
      let problem: string | null = null;
      if (!result) problem = `Tailscale isn't installed: ${DOWNLOAD}`;
      else if (result.timedOut) problem = `Tailscale is waiting for you. ${lastLines(result.stdout + result.stderr)}`;
      else if (result.code !== 0) problem = lastLines(result.stderr || result.stdout) || `tailscale serve failed with code ${result.code}.`;
      await refresh(problem);
    } finally {
      switching = false;
    }
  }

  return {
    status: () => status,
    onStatus(listener: () => void) {
      listeners.add(listener);
    },
    refresh: () => (switching ? Promise.resolve() : refresh()),
    setEnabled,
    gate: () => gate,
  };
}
