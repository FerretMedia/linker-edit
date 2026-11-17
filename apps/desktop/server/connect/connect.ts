import { randomUUID } from "node:crypto";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { LinkedComputer, LinkerConnect } from "../../shared/protocol.ts";
import type { LinkerService } from "../linker.ts";
import { startTunnel as realStartTunnel, type StartTunnel, type Tunnel, type TunnelEvent } from "./cloudflared.ts";
import type { Sessions } from "./sessions.ts";

/**
 * Linker Connect, copied from T3 Connect: your Linker account links up to 3 edit computers. Each gets an address
 * at Linker (https://edit-….linker.cool) that a Cloudflare Tunnel brings to this server on 127.0.0.1, so your
 * other computers reach it from anywhere, without Tailscale. Linker only makes and removes the tunnels
 * (app.linker.cool/api/edit/computers); the traffic goes straight through the tunnel to this computer.
 *
 * Getting in (server/http.ts): your other Linker-Edit shows this computer your Linker token, this computer asks
 * Linker whose it is, and only the account signed in here gets a one-time code for a session (sessions.ts).
 */

export const LIMIT = 3;
const LIST_FRESH_FOR = 30_000;
const NETWORK_TIMEOUT = 20_000;
/** Waits before starting a stopped tunnel again, growing up to the last. */
const RESTART_DELAYS = [2_000, 5_000, 15_000, 30_000, 60_000];
/** Sign-in attempts through the tunnel per minute, so nobody makes this computer hammer Linker. */
const ATTEMPTS_PER_MINUTE = 20;

interface Saved {
  computerId: string;
  enabled: boolean;
  url?: string;
  tunnelToken?: string;
  /** The Linker account that linked this computer, the only one let in. */
  ownerId?: string;
}

export interface ConnectOptions {
  /** ~/LinkerEdit/connect.json, readable only by this user: it holds the tunnel's token. */
  file: string;
  /** This server's port, which the tunnel forwards to. */
  port: number;
  linker: LinkerService;
  /** Linker's dashboard and API (app.linker.cool), where computers are linked. */
  appUrl: string;
  sessions: Sessions;
  /** Finds or downloads cloudflared; answers the file to run. */
  prepare: (onPercent: (percent: number) => void) => Promise<string>;
  startTunnel?: StartTunnel;
  fetch?: typeof fetch;
  name?: string;
  now?: () => number;
}

/** What Linker answers, worded for the user. */
class RelayError extends Error {
  readonly status: number;
  readonly body: Record<string, unknown>;
  constructor(status: number, body: Record<string, unknown>, message: string) {
    super(message);
    this.status = status;
    this.body = body;
  }
}

function relayMessage(status: number, body: Record<string, unknown>): string {
  if (status === 401) return "Linker didn't accept your sign-in. Sign out of Linker and in again.";
  if (status === 404 || status === 503) return "Linker Connect isn't available yet. Try again after Linker's next release.";
  if (body.error === "cloudflare") return `Linker couldn't set up the tunnel. Try again in a minute. (${String(body.message ?? "")})`;
  if (typeof body.message === "string" && body.message) return body.message;
  return `Linker answered ${status}.`;
}

function isComputer(value: unknown): value is LinkedComputer {
  const c = value as LinkedComputer | null;
  return !!c && typeof c.id === "string" && typeof c.name === "string" && typeof c.url === "string" && /^https:\/\//.test(c.url);
}

export type ConnectService = ReturnType<typeof createConnect>;

export function createConnect(options: ConnectOptions) {
  const fetchFn = options.fetch ?? fetch;
  const startTunnel = options.startTunnel ?? realStartTunnel;
  const now = options.now ?? Date.now;
  const name = options.name ?? os.hostname();
  const appUrl = options.appUrl.replace(/\/$/, "");
  const listeners = new Set<() => void>();
  const saved = read();

  let starting = false;
  let step: string | null = null;
  let problem: string | null = null;
  let connected = false;
  let tunnel: Tunnel | null = null;
  let restartTimer: NodeJS.Timeout | null = null;
  let restarts = 0;
  let computers: LinkedComputer[] | null = null;
  let listedAt = 0;
  let atLimit = false;
  let attempts: number[] = [];

  function read(): Saved {
    try {
      const value = JSON.parse(fs.readFileSync(options.file, "utf8")) as Partial<Saved>;
      if (typeof value.computerId === "string" && value.computerId) return { ...value, enabled: value.enabled === true } as Saved;
    } catch {
      // First run, or a damaged file: a new id, linked again when turned on.
    }
    return { computerId: randomUUID(), enabled: false };
  }

  function write() {
    fs.mkdirSync(path.dirname(options.file), { recursive: true });
    fs.writeFileSync(options.file, JSON.stringify(saved, null, 2), { mode: 0o600 });
  }

  function changed() {
    for (const listener of listeners) listener();
  }
  options.sessions.onChange(changed);

  const signedIn = () => options.linker.status().state === "signed_in";

  function status(): LinkerConnect {
    const state = !signedIn() && !tunnel ? "signed_out" : starting ? "starting" : tunnel ? "on" : "off";
    return {
      state,
      url: saved.url ?? null,
      connected: state === "on" && connected,
      step: starting ? step : null,
      problem,
      computerId: saved.computerId,
      computers,
      limit: LIMIT,
      atLimit,
      devices: options.sessions.list(),
    };
  }

  async function relay(method: string, route: string, body?: unknown): Promise<Record<string, unknown>> {
    const token = await options.linker.accessToken();
    if (!token) throw new RelayError(401, {}, "Sign in with Linker first.");
    const res = await fetchFn(`${appUrl}/api/edit/computers${route}`, {
      method,
      headers: { authorization: `Bearer ${token}`, accept: "application/json", ...(body ? { "content-type": "application/json" } : {}) },
      ...(body ? { body: JSON.stringify(body) } : {}),
      signal: AbortSignal.timeout(NETWORK_TIMEOUT),
    });
    const json = res.status === 204 ? {} : ((await res.json().catch(() => ({}))) as Record<string, unknown>);
    if (!res.ok) throw new RelayError(res.status, json, relayMessage(res.status, json));
    return json;
  }

  async function listComputers(): Promise<LinkedComputer[]> {
    const body = await relay("GET", "");
    computers = Array.isArray(body.computers) ? body.computers.filter(isComputer) : [];
    listedAt = now();
    return computers;
  }

  function clearRestart() {
    if (restartTimer) clearTimeout(restartTimer);
    restartTimer = null;
  }

  function stopTunnel() {
    clearRestart();
    tunnel?.stop();
    tunnel = null;
    connected = false;
  }

  function onTunnel(event: TunnelEvent) {
    if (event.state === "up") {
      connected = true;
      restarts = 0;
      problem = null;
    } else if (event.state === "connecting") {
      connected = false;
    } else {
      // cloudflared stopped by itself: start it again while Linker Connect is on.
      connected = false;
      tunnel = null;
      problem = event.detail ? `The tunnel stopped: ${event.detail}` : "The tunnel stopped.";
      if (saved.enabled && saved.tunnelToken) {
        const delay = RESTART_DELAYS[Math.min(restarts++, RESTART_DELAYS.length - 1)]!;
        clearRestart();
        restartTimer = setTimeout(() => void runTunnel(), delay);
      }
    }
    changed();
  }

  async function runTunnel() {
    clearRestart();
    if (!saved.tunnelToken || tunnel) return;
    step = "Getting cloudflared";
    changed();
    const file = await options.prepare((percent) => {
      step = `Downloading cloudflared · ${percent}%`;
      changed();
    });
    if (!saved.enabled || !saved.tunnelToken || tunnel) return;
    tunnel = startTunnel(file, saved.tunnelToken, onTunnel);
  }

  /** Links this computer at Linker, or refreshes its link; false when it couldn't. */
  async function link(): Promise<boolean> {
    if (!options.linker.userId()) await options.linker.refresh();
    const userId = options.linker.userId();
    if (!userId) {
      problem = "Linker didn't say who you are. Sign out of Linker and in again.";
      return false;
    }
    // Another account signed in here: the old link isn't theirs.
    if (saved.ownerId && saved.ownerId !== userId) {
      delete saved.url;
      delete saved.tunnelToken;
      saved.computerId = randomUUID();
    }
    step = "Linking this computer";
    changed();
    let body: Record<string, unknown>;
    try {
      body = await relay("PUT", `/${encodeURIComponent(saved.computerId)}`, { name: name.slice(0, 64), port: options.port });
    } catch (error) {
      if (error instanceof RelayError && error.status === 409 && error.body.error === "taken") {
        // Someone else's computer has this id, however unlikely: take a new one.
        saved.computerId = randomUUID();
        write();
        return link();
      }
      if (error instanceof RelayError && error.status === 409 && error.body.error === "limit") {
        atLimit = true;
        computers = Array.isArray(error.body.computers) ? error.body.computers.filter(isComputer) : computers;
        problem = `Your Linker account already has ${LIMIT} computers linked. Remove one below to link this one.`;
        return false;
      }
      throw error;
    }
    const computer = body.computer;
    if (!isComputer(computer) || typeof body.tunnelToken !== "string") throw new Error("Linker's answer was missing the tunnel.");
    saved.url = computer.url;
    saved.tunnelToken = body.tunnelToken;
    saved.ownerId = userId;
    atLimit = false;
    write();
    await listComputers().catch(() => {});
    return true;
  }

  async function turnOn() {
    if (starting || tunnel) return;
    starting = true;
    problem = null;
    changed();
    try {
      let linked = false;
      try {
        linked = await link();
      } catch (error) {
        // Offline at start: a computer linked before still opens its tunnel; Linker's side is unchanged.
        if (!(error instanceof RelayError) && saved.tunnelToken && saved.ownerId === options.linker.userId()) linked = true;
        problem = error instanceof Error ? error.message : String(error);
      }
      if (!linked) {
        saved.enabled = false;
        write();
        return;
      }
      saved.enabled = true;
      write();
      await runTunnel();
    } catch (error) {
      problem = error instanceof Error ? error.message : String(error);
      stopTunnel();
    } finally {
      starting = false;
      step = null;
      changed();
    }
  }

  function turnOff(reason: string | null) {
    stopTunnel();
    saved.enabled = false;
    write();
    // Nobody stays in through a tunnel that's off.
    options.sessions.revokeAll();
    problem = reason;
    changed();
  }

  // Signing out of Linker here ends Linker Connect: nobody can be checked against the account any more.
  options.linker.onStatus(() => {
    const state = options.linker.status().state;
    if (state === "signed_out" && (saved.enabled || tunnel)) turnOff("Linker Connect turned off when you signed out of Linker.");
    else changed();
  });

  return {
    status,
    onStatus(listener: () => void) {
      listeners.add(listener);
    },

    /** At start: turns Linker Connect back on when it was on. */
    start() {
      if (saved.enabled && signedIn()) void turnOn();
    },

    async setEnabled(enabled: boolean) {
      if (enabled) await turnOn();
      else turnOff(null);
    },

    /** Looks up your linked computers again, for the settings page. */
    async refresh() {
      if (!signedIn()) return;
      try {
        await listComputers();
      } catch (error) {
        problem = error instanceof Error ? error.message : String(error);
      }
      changed();
    },

    /** Unlinks one of your computers at Linker; for this one, also turns Linker Connect off here. */
    async unlink(computerId: string) {
      try {
        await relay("DELETE", `/${encodeURIComponent(computerId)}`);
      } catch (error) {
        // Already gone is what was asked.
        if (!(error instanceof RelayError && error.status === 404)) {
          problem = error instanceof Error ? error.message : String(error);
          changed();
          return;
        }
      }
      if (computerId === saved.computerId) {
        delete saved.url;
        delete saved.tunnelToken;
        turnOff(null);
      }
      atLimit = false;
      problem = null;
      await listComputers().catch(() => {});
      changed();
    },

    signOutDevice(deviceId: string) {
      options.sessions.revoke(deviceId);
    },

    /** This computer's public address host while Linker Connect is on, for server/http.ts. */
    host(): string | null {
      if (!tunnel || !saved.url) return null;
      try {
        return new URL(saved.url).host.toLowerCase();
      } catch {
        return null;
      }
    },

    /**
     * Someone at the tunnel shows a Linker token. Only the account that linked this computer gets a one-time
     * code for a session.
     */
    async admit(token: string, device: string): Promise<{ code: string } | { status: number; error: string }> {
      if (!tunnel || !saved.ownerId) return { status: 503, error: "Linker Connect is off on this computer." };
      attempts = attempts.filter((at) => now() - at < 60_000);
      if (attempts.length >= ATTEMPTS_PER_MINUTE) return { status: 429, error: "Too many tries. Wait a minute." };
      attempts.push(now());
      let who: string | null;
      try {
        who = await options.linker.whoIs(token);
      } catch {
        return { status: 502, error: `${name} couldn't reach Linker to check your sign-in. Try again.` };
      }
      if (!who) return { status: 401, error: "Linker didn't accept that sign-in. Sign out of Linker and in again." };
      if (who !== saved.ownerId || who !== options.linker.userId()) {
        return { status: 403, error: `${name} belongs to another Linker account. Sign in with the same account on both computers.` };
      }
      return { code: options.sessions.issueCode(device) };
    },

    /** Your other linked computers, for the computer menu here; empty when signed out or Linker is unreachable. */
    async others(): Promise<LinkedComputer[]> {
      if (!signedIn()) return [];
      try {
        const list = computers && now() - listedAt < LIST_FRESH_FOR ? computers : await listComputers();
        return list.filter((computer) => computer.id !== saved.computerId);
      } catch (error) {
        console.error(`Couldn't list Linker Connect computers: ${error instanceof Error ? error.message : String(error)}`);
        return [];
      }
    },

    /**
     * Opening another of your computers from here: asks it for a one-time code with this computer's Linker
     * sign-in, and answers the address that swaps the code for a session.
     */
    async open(url: string): Promise<string> {
      const origin = new URL(url).origin;
      const find = (list: LinkedComputer[] | null) => list?.find((computer) => new URL(computer.url).origin === origin);
      const computer = find(computers) ?? find(await listComputers());
      if (!computer) throw new Error("That computer isn't linked to your Linker account.");
      const token = await options.linker.accessToken();
      if (!token) throw new Error("Sign in with Linker on this computer first.");
      let res: Response;
      try {
        res = await fetchFn(`${origin}/api/connect/session`, {
          method: "POST",
          headers: { authorization: `Bearer ${token}`, "content-type": "application/json", accept: "application/json" },
          body: JSON.stringify({ name }),
          signal: AbortSignal.timeout(NETWORK_TIMEOUT),
        });
      } catch {
        throw new Error(`Couldn't reach ${computer.name}. Is it on, with Linker-Edit running and Linker Connect on?`);
      }
      const body = (await res.json().catch(() => ({}))) as { code?: unknown; error?: unknown };
      if (!res.ok || typeof body.code !== "string") {
        throw new Error(typeof body.error === "string" ? body.error : `${computer.name} answered ${res.status}.`);
      }
      return `${origin}/api/connect/enter?code=${encodeURIComponent(body.code)}`;
    },
  };
}
