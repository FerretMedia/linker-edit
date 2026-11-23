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
 * Getting in (server/http.ts): your other Linker-Edit, the iPad app or a page on your own web address shows this
 * computer your Linker token, this computer asks Linker whose it is, and only the account signed in here gets a
 * one-time code for a session (sessions.ts): a cookie for Linker-Edit's window, a Bearer token for the others
 * (redeem). That works through the tunnel and over Tailscale; "Use the tunnel" off links this computer for
 * Tailscale only, with no tunnel at all.
 */

export const LIMIT = 3;
const LIST_FRESH_FOR = 30_000;
const NETWORK_TIMEOUT = 20_000;
/** A kept session on another computer is used again only while it has more than this left. */
const KEPT_SESSION_MARGIN = 24 * 60 * 60_000;
/** Waits before starting a stopped tunnel again, growing up to the last. */
const RESTART_DELAYS = [2_000, 5_000, 15_000, 30_000, 60_000];
/** Sign-in attempts through the tunnel per minute, each way in, so nobody makes this computer hammer Linker. */
const ATTEMPTS_PER_MINUTE = 20;

/** True while fewer than ATTEMPTS_PER_MINUTE tries came in the last minute, counting this one. */
function rateLimit(now: () => number) {
  let attempts: number[] = [];
  return () => {
    attempts = attempts.filter((at) => now() - at < 60_000);
    if (attempts.length >= ATTEMPTS_PER_MINUTE) return false;
    attempts.push(now());
    return true;
  };
}

interface Saved {
  computerId: string;
  enabled: boolean;
  url?: string;
  tunnelToken?: string;
  /** "Use the tunnel"; missing in files from before it, which all did. */
  tunnel?: boolean;
  /** The Tailscale address last sent to Linker. */
  tailnetUrl?: string | null;
  /** The Linker account that linked this computer, the only one let in. */
  ownerId?: string;
  /**
   * This computer's sessions on your other computers, by their origin, kept so a restart doesn't sign in again and
   * add another device there. Only used while the same Linker account (`by`) is signed in here.
   */
  outgoing?: Record<string, { session: string; expiresAt: string; by: string }>;
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
  /**
   * This computer's Tailscale remote access address while it's on, which Linker lists for your web address;
   * null while it's off, undefined while not known yet.
   */
  tailnetUrl?: () => string | null | undefined;
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

const httpsOrNull = (value: unknown) => (typeof value === "string" && /^https:\/\//.test(value) ? value : null);

/** A computer from Linker's answer, or null when it isn't one; url is null for a computer linked without a tunnel. */
function toComputer(value: unknown): LinkedComputer | null {
  const c = value as Record<string, unknown> | null;
  if (!c || typeof c.id !== "string" || typeof c.name !== "string") return null;
  const url = httpsOrNull(c.url);
  if (url === null && c.url !== null) return null;
  return { ...(c as unknown as LinkedComputer), url, tailnetUrl: httpsOrNull(c.tailnetUrl) };
}

const toComputers = (list: unknown): LinkedComputer[] | null => (Array.isArray(list) ? list.flatMap((entry) => toComputer(entry) ?? []) : null);

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
  const admitting = rateLimit(now);
  const redeeming = rateLimit(now);

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
  const useTunnel = () => saved.tunnel !== false;
  /** Linked and on: with the tunnel running, or for Tailscale only. */
  const on = () => tunnel !== null || (saved.enabled && !useTunnel() && !!saved.ownerId);

  function status(): LinkerConnect {
    const state = !signedIn() && !tunnel ? "signed_out" : starting ? "starting" : on() ? "on" : "off";
    return {
      state,
      tunnel: useTunnel(),
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
    computers = toComputers(body.computers) ?? [];
    listedAt = now();
    return computers;
  }

  /**
   * Asks another of your computers, at its tunnel address, for a one-time code with this computer's Linker sign-in;
   * null when this computer isn't signed in to Linker. Only a computer linked to the account gets the token.
   */
  async function codeFrom(url: string): Promise<{ origin: string; computer: LinkedComputer; code: string } | null> {
    const origin = new URL(url).origin;
    const find = (list: LinkedComputer[] | null) => list?.find((computer) => computer.url !== null && new URL(computer.url).origin === origin);
    const token = await options.linker.accessToken();
    if (!token) return null;
    const computer = find(computers) ?? find(await listComputers());
    if (!computer) throw new Error("That computer isn't linked to your Linker account.");
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
    return { origin, computer, code: body.code };
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
    // Another account signed in here: the old link isn't theirs, nor are the devices the old one signed in.
    if (saved.ownerId && saved.ownerId !== userId) {
      delete saved.url;
      delete saved.tunnelToken;
      saved.computerId = randomUUID();
      options.sessions.revokeAll();
    }
    step = "Linking this computer";
    changed();
    // Not known yet (Tailscale still being checked): what Linker was told last.
    const known = options.tailnetUrl?.();
    const tailnetUrl = known === undefined ? (saved.tailnetUrl ?? null) : known;
    let body: Record<string, unknown>;
    try {
      body = await relay("PUT", `/${encodeURIComponent(saved.computerId)}`, {
        name: name.slice(0, 64),
        port: options.port,
        tailnetUrl,
        tunnel: useTunnel(),
      });
    } catch (error) {
      if (error instanceof RelayError && error.status === 409 && error.body.error === "taken") {
        // Someone else's computer has this id, however unlikely: take a new one.
        saved.computerId = randomUUID();
        write();
        return link();
      }
      if (error instanceof RelayError && error.status === 409 && error.body.error === "limit") {
        atLimit = true;
        computers = toComputers(error.body.computers) ?? computers;
        problem = `Your Linker account already has ${LIMIT} computers linked. Remove one below to link this one.`;
        return false;
      }
      throw error;
    }
    const computer = toComputer(body.computer);
    if (!computer) throw new Error("Linker's answer was missing this computer.");
    if (useTunnel()) {
      if (!computer.url || typeof body.tunnelToken !== "string") throw new Error("Linker's answer was missing the tunnel.");
      // Linker gave the tunnel a new token: cloudflared starts again with it.
      const restart = tunnel !== null && saved.tunnelToken !== body.tunnelToken;
      saved.url = computer.url;
      saved.tunnelToken = body.tunnelToken;
      if (restart) {
        stopTunnel();
        void runTunnel().catch(() => {});
      }
    } else {
      delete saved.url;
      delete saved.tunnelToken;
    }
    saved.tailnetUrl = tailnetUrl;
    saved.ownerId = userId;
    atLimit = false;
    write();
    await listComputers().catch(() => {});
    return true;
  }

  async function turnOn() {
    if (starting || on()) return;
    starting = true;
    problem = null;
    changed();
    try {
      let linked = false;
      try {
        linked = await link();
      } catch (error) {
        // Offline at start: a computer linked before still opens its tunnel; Linker's side is unchanged.
        const linkedBefore = useTunnel() ? !!saved.tunnelToken : !!saved.ownerId;
        if (!(error instanceof RelayError) && linkedBefore && saved.ownerId === options.linker.userId()) linked = true;
        problem = error instanceof Error ? error.message : String(error);
      }
      if (!linked) {
        saved.enabled = false;
        write();
        return;
      }
      saved.enabled = true;
      write();
      if (useTunnel()) await runTunnel();
    } catch (error) {
      problem = error instanceof Error ? error.message : String(error);
      stopTunnel();
    } finally {
      starting = false;
      step = null;
      changed();
    }
    // Tailscale's remote access may have changed while this was linking.
    tailnetChanged();
  }

  /** Tells Linker this computer's Tailscale address again when remote access was turned on or off while linked. */
  function tailnetChanged() {
    const next = options.tailnetUrl?.();
    if (next === undefined || next === (saved.tailnetUrl ?? null) || !saved.enabled || starting || !signedIn()) return;
    link()
      .catch((error: unknown) => void (problem = error instanceof Error ? error.message : String(error)))
      .finally(changed);
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

    /**
     * "Use the tunnel": off links this computer for Tailscale only, and Linker removes its tunnel. Devices signed in
     * stay signed in; they come in over Tailscale now.
     */
    async setTunnel(value: boolean) {
      if (useTunnel() === value || starting) return;
      saved.tunnel = value;
      write();
      if (!saved.enabled) {
        changed();
        return;
      }
      if (!value) stopTunnel();
      starting = true;
      problem = null;
      changed();
      try {
        if ((await link()) && value) await runTunnel();
      } catch (error) {
        problem = error instanceof Error ? error.message : String(error);
      } finally {
        starting = false;
        step = null;
        changed();
      }
    },

    tailnetChanged,

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
     * Someone at the tunnel or on Tailscale shows a Linker token (a web token from your own web address too).
     * Only the account that linked this computer gets a one-time code for a session.
     */
    async admit(token: string, device: string): Promise<{ code: string } | { status: number; error: string }> {
      if (!on() || !saved.ownerId) return { status: 503, error: "Linker Connect is off on this computer." };
      if (!admitting()) return { status: 429, error: "Too many tries. Wait a minute." };
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

    /**
     * A native app (the iPad app) swaps the one-time code admit gave it for a session it sends as a Bearer token.
     * The code only exists on this computer, so it can't be used at another.
     */
    redeem(code: string): { session: string; expiresAt: string } | { status: number; error: string } {
      if (!on() || !saved.ownerId) return { status: 503, error: "Linker Connect is off on this computer." };
      if (!redeeming()) return { status: 429, error: "Too many tries. Wait a minute." };
      const entered = code ? options.sessions.enter(code, "native") : null;
      if (!entered) return { status: 401, error: "That code ran out or was used already. Sign in again." };
      return { session: entered.token, expiresAt: new Date(entered.expiresAt).toISOString() };
    },

    /**
     * Your other linked computers, for the computer menu here, asked of Linker again when fresh; empty when signed out
     * or Linker is unreachable.
     */
    async others(fresh = false): Promise<LinkedComputer[]> {
      if (!signedIn()) return [];
      try {
        const list = !fresh && computers && now() - listedAt < LIST_FRESH_FOR ? computers : await listComputers();
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
      const got = await codeFrom(url);
      if (!got) throw new Error("Sign in with Linker on this computer first.");
      return `${got.origin}/api/connect/enter?code=${encodeURIComponent(got.code)}`;
    },

    /**
     * A Bearer session on another of your computers, for this computer's server to pass the window's requests on
     * with (server/machines.ts), the way the iPad app signs in; null when this computer isn't signed in to Linker.
     * The one kept from before is used again, unless it's `failed`: the other computer just turned it down.
     */
    async sessionOn(url: string, failed: string | null = null): Promise<{ session: string; expiresAt: string } | null> {
      const origin = new URL(url).origin;
      const user = options.linker.userId();
      const kept = saved.outgoing?.[origin];
      if (kept && user && kept.by === user && kept.session !== failed && Date.parse(kept.expiresAt) - now() > KEPT_SESSION_MARGIN) {
        return { session: kept.session, expiresAt: kept.expiresAt };
      }
      const got = await codeFrom(url);
      if (!got) return null;
      let res: Response;
      try {
        res = await fetchFn(`${got.origin}/api/connect/native`, {
          method: "POST",
          headers: { "content-type": "application/json", accept: "application/json" },
          body: JSON.stringify({ code: got.code }),
          signal: AbortSignal.timeout(NETWORK_TIMEOUT),
        });
      } catch {
        throw new Error(`Couldn't reach ${got.computer.name}. Is it on, with Linker-Edit running and Linker Connect on?`);
      }
      const body = (await res.json().catch(() => ({}))) as { session?: unknown; expiresAt?: unknown; error?: unknown };
      if (!res.ok || typeof body.session !== "string" || typeof body.expiresAt !== "string") {
        throw new Error(typeof body.error === "string" ? body.error : `${got.computer.name} answered ${res.status}.`);
      }
      if (user) {
        const outgoing = Object.fromEntries(
          Object.entries(saved.outgoing ?? {}).filter(([, other]) => other.by === user && Date.parse(other.expiresAt) > now()),
        );
        saved.outgoing = { ...outgoing, [origin]: { session: body.session, expiresAt: body.expiresAt, by: user } };
        write();
      }
      return { session: body.session, expiresAt: body.expiresAt };
    },
  };
}
