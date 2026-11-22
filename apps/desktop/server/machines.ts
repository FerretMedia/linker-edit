import * as http from "node:http";
import * as https from "node:https";
import type { Duplex } from "node:stream";
import { WebSocket, WebSocketServer, type RawData } from "ws";
import type { Computer, LinkedComputer, Machine, Machines } from "../shared/protocol.ts";
import type { Probe } from "./remote.ts";

/**
 * The user's other computers in the desktop app's sidebar, like T3 Code's environments. The window only ever talks
 * to this computer's server; requests to /machines/<id>/… are passed on to that computer here, signed in with this
 * computer's own sign-in: over Tailscale the owner's devices get in as they are, through a Linker Connect tunnel
 * with a Bearer session got the way the iPad app gets one. The page's Origin and cookies never go along, so the
 * other computer treats these requests like its native apps', not like a web page's.
 */

const LIST_FRESH_FOR = 30_000;
/** A request body up to this size is kept, so the request can be sent again with a new session or another way. */
const REPLAYABLE = 1024 * 1024;
const CONNECT_TIMEOUT = 10_000;
/** What the window's WebSocket may ask for; see server/http.ts. */
const WS_PROTOCOL = "linker-edit";

/** One way to reach a computer: its Tailscale address, or its Linker Connect tunnel, which needs a session. */
interface Route {
  origin: string;
  via: "tailscale" | "linker";
}

interface Found {
  machine: Machine;
  /** Tried in order: Tailscale first, the direct path, then the tunnel. */
  routes: Route[];
}

export interface MachinesOptions {
  name: string;
  version: string;
  /** Your other Linker Connect computers, asked of Linker again when fresh. */
  linked: (fresh: boolean) => Promise<LinkedComputer[]>;
  /** Your other computers that answer over Tailscale. */
  tailscale: () => Promise<Computer[]>;
  probe: Probe;
  /** A Bearer session at a Linker Connect computer's tunnel address; null when not signed in to Linker here. */
  sessionOn: (url: string) => Promise<{ session: string; expiresAt: string } | null>;
  now?: () => number;
}

export type MachinesService = ReturnType<typeof createMachines>;

const originOf = (url: string) => {
  try {
    return new URL(url).origin;
  } catch {
    return null;
  }
};

const baseOf = (id: string) => `/machines/${encodeURIComponent(id)}`;

/** Never passed on: the page's own sign-ins and where it's from, and what only one connection means. */
const DROP_REQUEST = new Set([
  "host",
  "origin",
  "referer",
  "cookie",
  "authorization",
  "connection",
  "keep-alive",
  "proxy-authorization",
  "proxy-connection",
  "te",
  "trailer",
  "transfer-encoding",
  "upgrade",
  "content-length",
]);
/** The other computer's cookies are for its own pages, never this one's. */
const DROP_RESPONSE = new Set(["connection", "keep-alive", "transfer-encoding", "set-cookie"]);

/** A redirect to a path on the other computer stays under /machines/<id>. */
export function machineLocation(location: string, origin: string, base: string): string {
  if (location.startsWith("/") && !location.startsWith("//")) return base + location;
  try {
    const url = new URL(location);
    return url.origin === origin ? base + url.pathname + url.search + url.hash : location;
  } catch {
    return location;
  }
}

/** A request body read up to limit bytes: all of it when done, else the start, with the rest still in req. */
function readUpTo(req: http.IncomingMessage, limit: number): Promise<{ chunks: Buffer[]; done: boolean }> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    const onData = (chunk: Buffer) => {
      chunks.push(chunk);
      size += chunk.length;
      if (size > limit) {
        req.pause();
        req.off("data", onData);
        req.off("end", onEnd);
        resolve({ chunks, done: false });
      }
    };
    const onEnd = () => resolve({ chunks, done: true });
    req.on("data", onData);
    req.on("end", onEnd);
    req.on("error", reject);
  });
}

/** A network failure, which another way to the computer may not have. */
class Unreachable extends Error {}

/** Writes a JSON error on a socket that asked to become a WebSocket. */
function refuse(socket: Duplex, status: number, error: string) {
  const body = JSON.stringify({ error });
  socket.end(
    `HTTP/1.1 ${status} ${http.STATUS_CODES[status]}\r\ncontent-type: application/json\r\ncontent-length: ${Buffer.byteLength(body)}\r\nconnection: close\r\n\r\n${body}`,
  );
}

/** Splits /machines/<id>/<rest> into the computer's id and the path on it; null when it isn't one. */
function parse(rawUrl: string | undefined): { id: string; rest: string } | null {
  const match = /^\/machines\/([^/?#]+)(.*)$/s.exec(rawUrl ?? "");
  if (!match) return null;
  try {
    const rest = match[2] || "/";
    return { id: decodeURIComponent(match[1]!), rest: rest.startsWith("/") ? rest : `/${rest}` };
  } catch {
    return null;
  }
}

const NOT_FOUND = "That computer isn't one of yours, or isn't linked any more.";
const unreachable = (machine: Machine) => `Couldn't reach ${machine.name}. Is it on, with Linker-Edit running?`;
const signedOut = (machine: Machine) => `Sign in with Linker on this computer to reach ${machine.name}.`;

export function createMachines(options: MachinesOptions) {
  const now = options.now ?? Date.now;
  let found: Promise<Map<string, Found>> | null = null;
  let foundAt = 0;
  const sessions = new Map<string, Promise<{ session: string; expiresAt: number } | null>>();

  async function look(fresh: boolean): Promise<Map<string, Found>> {
    const [linked, peers] = await Promise.all([options.linked(fresh), options.tailscale().catch(() => [] as Computer[])]);
    const byOrigin = new Map<string, Computer>();
    for (const peer of peers) {
      const origin = originOf(peer.url);
      if (origin) byOrigin.set(origin, peer);
    }
    const viaLinker = await Promise.all(
      linked.map(async (computer): Promise<Found | null> => {
        const base = baseOf(computer.id);
        const tunnel = computer.url ? originOf(computer.url) : null;
        const tunnelRoute: Route[] = tunnel ? [{ origin: tunnel, via: "linker" }] : [];
        const tailnet = computer.tailnetUrl ? originOf(computer.tailnetUrl) : null;
        const peer = tailnet ? byOrigin.get(tailnet) : undefined;
        // The same computer over Tailscale: one entry, reached directly, with the tunnel to fall back on.
        if (tailnet && peer) {
          byOrigin.delete(tailnet);
          const machine: Machine = { id: computer.id, name: peer.name || computer.name, version: peer.version, via: "tailscale", online: true, base };
          return { machine, routes: [{ origin: tailnet, via: "tailscale" }, ...tunnelRoute] };
        }
        if (!tunnel) return null;
        const hello = computer.online === false ? null : await options.probe(tunnel);
        const machine: Machine = { id: computer.id, name: hello?.name || computer.name, version: hello?.version ?? "", via: "linker", online: hello !== null, base };
        return { machine, routes: tunnelRoute };
      }),
    );
    const viaTailscale = [...byOrigin].map(([origin, peer]): Found => {
      const id = `tailscale-${new URL(origin).hostname}`;
      return { machine: { id, name: peer.name, version: peer.version, via: "tailscale", online: true, base: baseOf(id) }, routes: [{ origin, via: "tailscale" }] };
    });
    return new Map([...viaLinker.flatMap((entry) => entry ?? []), ...viaTailscale].map((entry) => [entry.machine.id, entry]));
  }

  function current(fresh = false): Promise<Map<string, Found>> {
    if (fresh || !found || now() - foundAt >= LIST_FRESH_FOR) {
      const next = look(fresh);
      found = next;
      foundAt = now();
      next.catch(() => {
        if (found === next) found = null;
      });
    }
    return found!;
  }

  /** A computer for the proxy: from the last list at once, looked up again behind it when that's old. */
  async function find(id: string): Promise<Found | null> {
    const last = found;
    if (last && now() - foundAt >= LIST_FRESH_FOR) void current().catch(() => {});
    const list = await (last ?? current()).catch(() => current());
    return list.get(id) ?? null;
  }

  /**
   * This computer's session at a tunnel address, asked for once and kept. failed is one the other computer just
   * turned down: a new one is asked for, once however many requests found out at the same time.
   */
  async function sessionAt(origin: string, failed: string | null = null): Promise<string | null> {
    const kept = sessions.get(origin);
    if (kept) {
      const got = await kept.catch(() => null);
      if (got && got.session !== failed && got.expiresAt > now()) return got.session;
      if (sessions.get(origin) !== kept) return sessionAt(origin, failed);
    }
    const next = options.sessionOn(origin).then((got) => got && { session: got.session, expiresAt: Date.parse(got.expiresAt) });
    sessions.set(origin, next);
    const forget = () => {
      if (sessions.get(origin) === next) sessions.delete(origin);
    };
    next.then((got) => got ?? forget(), forget);
    return (await next)?.session ?? null;
  }

  function send(route: Route, session: string | null, req: http.IncomingMessage, rest: string, body: { chunks: Buffer[]; done: boolean }) {
    return new Promise<http.IncomingMessage>((resolve, reject) => {
      const headers: http.OutgoingHttpHeaders = {};
      for (const [name, value] of Object.entries(req.headers)) if (!DROP_REQUEST.has(name) && value !== undefined) headers[name] = value;
      if (session) headers.authorization = `Bearer ${session}`;
      const length = body.chunks.reduce((sum, chunk) => sum + chunk.length, 0);
      if (body.done && (length > 0 || req.headers["content-length"] !== undefined)) headers["content-length"] = length;
      else if (!body.done && req.headers["content-length"] !== undefined) headers["content-length"] = req.headers["content-length"];
      const url = new URL(rest, route.origin);
      const upstream = (url.protocol === "https:" ? https : http).request(url, { method: req.method, headers });
      // Only reaching the computer is timed: a video streams for as long as it plays.
      const timer = setTimeout(() => upstream.destroy(new Unreachable("timed out")), CONNECT_TIMEOUT);
      upstream.on("socket", (socket) => {
        if (!socket.connecting) clearTimeout(timer);
        else socket.once("connect", () => clearTimeout(timer));
      });
      upstream.on("response", (res) => {
        clearTimeout(timer);
        resolve(res);
      });
      upstream.on("error", (error) => {
        clearTimeout(timer);
        reject(error instanceof Unreachable ? error : new Unreachable(error.message));
      });
      for (const chunk of body.chunks) upstream.write(chunk);
      if (body.done) upstream.end();
      else req.pipe(upstream);
    });
  }

  /** Passes a request from the window on to the computer it names; server/http.ts says who may. */
  async function proxy(req: http.IncomingMessage, res: http.ServerResponse): Promise<void> {
    const json = (status: number, body: unknown) =>
      res.writeHead(status, { "content-type": "application/json", "cache-control": "no-store" }).end(JSON.stringify(body));
    const target = parse(req.url);
    const entry = target ? await find(target.id) : null;
    if (!target || !entry) {
      json(404, { error: NOT_FOUND });
      return;
    }
    const { machine, routes } = entry;
    const body = await readUpTo(req, REPLAYABLE);
    let problem = unreachable(machine);
    for (const [index, route] of routes.entries()) {
      let session: string | null = null;
      let upstream: http.IncomingMessage;
      try {
        if (route.via === "linker") session = await sessionAt(route.origin);
        if (route.via === "linker" && !session) {
          json(401, { error: signedOut(machine) });
          return;
        }
        upstream = await send(route, session, req, target.rest, body);
        // The other computer signed this one out: a new session, and the request again when it can be sent again.
        if (upstream.statusCode === 401 && session && body.done) {
          upstream.resume();
          session = await sessionAt(route.origin, session);
          if (!session) {
            json(401, { error: signedOut(machine) });
            return;
          }
          upstream = await send(route, session, req, target.rest, body);
        }
      } catch (error) {
        if (!(error instanceof Unreachable)) problem = error instanceof Error ? error.message : String(error);
        // The list is looked at again soon; meanwhile the next way there, when the request can be sent again.
        foundAt = 0;
        if (body.done && index < routes.length - 1) continue;
        break;
      }
      const headers: http.OutgoingHttpHeaders = {};
      for (const [name, value] of Object.entries(upstream.headers)) if (!DROP_RESPONSE.has(name) && value !== undefined) headers[name] = value;
      if (typeof headers.location === "string") headers.location = machineLocation(headers.location, route.origin, machine.base);
      res.writeHead(upstream.statusCode ?? 502, headers);
      upstream.on("error", () => res.destroy());
      res.on("close", () => {
        if (!res.writableFinished) upstream.destroy();
      });
      upstream.pipe(res);
      return;
    }
    json(502, { error: problem });
  }

  // The window asks for this subprotocol in desktop mode as on the web; the other computer never sees it.
  const pages = new WebSocketServer({ noServer: true, handleProtocols: (offered) => (offered.has(WS_PROTOCOL) ? WS_PROTOCOL : false) });

  /** Opens a socket to the other computer: open (and paused), turned away with a status, or not reached (null). */
  function openUpstream(route: Route, session: string | null, rest: string): Promise<{ ws: WebSocket } | { status: number } | null> {
    return new Promise((resolve) => {
      const url = new URL(rest, route.origin);
      url.protocol = url.protocol === "https:" ? "wss:" : "ws:";
      // A Bearer session goes in a header, as the iPad app sends it. Over Tailscale the owner's own pages get in by
      // naming the other computer's own address.
      const ws = new WebSocket(url, {
        handshakeTimeout: CONNECT_TIMEOUT,
        ...(session ? { headers: { authorization: `Bearer ${session}` } } : { origin: route.origin }),
      });
      ws.on("open", () => {
        ws.pause();
        resolve({ ws });
      });
      ws.on("unexpected-response", (request, response) => {
        response.resume();
        request.destroy();
        resolve({ status: response.statusCode ?? 502 });
      });
      ws.on("error", () => resolve(null));
    });
  }

  /** Joins the window's /machines/<id>/ws to the other computer's /ws: messages both ways, closed together. */
  async function upgrade(req: http.IncomingMessage, socket: Duplex, head: Buffer): Promise<void> {
    let gone = false;
    socket.once("close", () => (gone = true));
    socket.on("error", () => {});
    const target = parse(req.url);
    const entry = target ? await find(target.id) : null;
    if (!target || !entry) {
      refuse(socket, 404, NOT_FOUND);
      return;
    }
    const { machine, routes } = entry;
    let upstream: WebSocket | null = null;
    let status = 502;
    let problem = unreachable(machine);
    for (const route of routes) {
      let session: string | null = null;
      try {
        if (route.via === "linker") session = await sessionAt(route.origin);
      } catch (error) {
        problem = error instanceof Error ? error.message : String(error);
        continue;
      }
      if (route.via === "linker" && !session) {
        refuse(socket, 401, signedOut(machine));
        return;
      }
      let opened = await openUpstream(route, session, target.rest);
      // Signed out there: once more with a new session.
      if (opened && "status" in opened && opened.status === 401 && session) {
        session = await sessionAt(route.origin, session).catch(() => null);
        opened = session ? await openUpstream(route, session, target.rest) : opened;
      }
      if (opened === null) {
        foundAt = 0;
        continue;
      }
      if ("ws" in opened) upstream = opened.ws;
      else {
        status = opened.status;
        problem = `${machine.name} turned the connection down (${opened.status}).`;
      }
      break;
    }
    if (!upstream) {
      refuse(socket, status, problem);
      return;
    }
    if (gone) {
      upstream.close();
      return;
    }
    const remote = upstream;
    pages.handleUpgrade(req, socket, head, (page) => {
      const relay = (to: WebSocket) => (data: RawData, isBinary: boolean) => {
        if (to.readyState === WebSocket.OPEN) to.send(data, { binary: isBinary });
      };
      const closeAlso = (other: WebSocket) => (code: number, reason: Buffer) => {
        if (other.readyState !== WebSocket.OPEN && other.readyState !== WebSocket.CONNECTING) return;
        try {
          other.close(code, reason);
        } catch {
          // 1005 and 1006 only describe a close; they can't be sent.
          other.close();
        }
      };
      page.on("message", relay(remote));
      remote.on("message", relay(page));
      page.on("close", closeAlso(remote));
      remote.on("close", closeAlso(page));
      page.on("error", () => remote.terminate());
      remote.on("error", () => page.terminate());
      remote.resume();
    });
  }

  return {
    /** This computer first, then the others; looked up again after 30 seconds, or at once when fresh. */
    async list(fresh = false): Promise<Machines> {
      const others = [...(await current(fresh)).values()].map((entry) => entry.machine);
      const self: Machine = { id: "this", name: options.name, version: options.version, via: "this", online: true, base: "" };
      return { machines: [self, ...others] };
    },
    proxy,
    upgrade,
  };
}
