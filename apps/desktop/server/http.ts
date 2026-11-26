import * as fs from "node:fs";
import * as http from "node:http";
import * as os from "node:os";
import * as path from "node:path";
import type { Duplex } from "node:stream";
import { promisify } from "node:util";
import * as zlib from "node:zlib";
import { WebSocketServer, type WebSocket } from "ws";
import { PROTOCOL_VERSION, type ClientMessage, type Computer, type Hello, type LinkedComputer } from "../shared/protocol.ts";
import type { App } from "./app.ts";
import type { LinkerService } from "./linker.ts";
import type { GoogleAuth } from "./drive/auth.ts";
import type { ConnectService } from "./connect/connect.ts";
import type { Sessions } from "./connect/sessions.ts";
import type { WebOrigins } from "./connect/webOrigins.ts";
import { createMachines } from "./machines.ts";
import { probeLinkerEdit, requestSource, type Probe, type RemoteGate } from "./remote.ts";
import { parseUsageProviders, parseUsageWindow } from "./usage/summary.ts";

const MIME: Record<string, string> = {
  ".html": "text/html",
  ".js": "text/javascript",
  ".css": "text/css",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".json": "application/json",
  ".woff2": "font/woff2",
};

const LOOPBACK_HOSTS = new Set(["localhost", "127.0.0.1", "[::1]"]);

const COMPRESSIBLE = new Set([".html", ".js", ".css", ".svg", ".json"]);

// The built UI, gzipped once per file and kept: about a quarter of the size, for pages opened from another device.
const gzipped = new Map<string, { mtimeMs: number; data: Promise<Buffer> }>();
const gzip = promisify(zlib.gzip);
function gzippedFile(file: string): Promise<Buffer> {
  const { mtimeMs } = fs.statSync(file);
  const cached = gzipped.get(file);
  if (cached?.mtimeMs === mtimeMs) return cached.data;
  const data = fs.promises.readFile(file).then((raw) => gzip(raw, { level: 9 }));
  data.catch(() => gzipped.delete(file));
  gzipped.set(file, { mtimeMs, data });
  return data;
}

/** Whether an Accept-Encoding header takes gzip ("gzip;q=0" refuses it). */
export const acceptsGzip = (header: string | undefined) =>
  (header ?? "").split(",").some((part) => {
    const [name = "", ...params] = part.split(";").map((piece) => piece.trim());
    return name === "gzip" && !params.some((param) => /^q=0(\.0*)?$/.test(param));
  });

/**
 * The UI's file names under _app/immutable change with their contents, so a browser keeps them; the page itself is
 * always checked, so a new release shows at once.
 */
export const staticCacheControl = (urlPath: string) => (urlPath.startsWith("/_app/immutable/") ? "public, max-age=31536000, immutable" : "no-cache");

/**
 * Browsers let any website open a WebSocket to localhost, and this socket drives an agent that runs
 * commands. Only this app's own pages may connect: same origin as the request, which also turns away
 * other apps served on localhost. Non-browser clients send no Origin.
 */
export function isAllowedOrigin(origin: string | undefined, host: string | undefined): boolean {
  if (origin === undefined) return true;
  try {
    const url = new URL(origin);
    return (
      (url.protocol === "http:" || url.protocol === "https:") && LOOPBACK_HOSTS.has(url.hostname) && url.host === host?.toLowerCase()
    );
  } catch {
    return false;
  }
}

/** The built file for a URL path, index.html for client-side routes, or null when the UI isn't built. */
export function staticFile(distDir: string, urlPath: string): string | null {
  const file = path.join(distDir, urlPath);
  const relative = path.relative(distDir, file);
  const inside = relative !== "" && relative.split(path.sep)[0] !== ".." && !path.isAbsolute(relative);
  if (inside && fs.existsSync(file) && fs.statSync(file).isFile()) return file;
  const index = path.join(distDir, "index.html");
  return fs.existsSync(index) ? index : null;
}

/** The bytes a Range header asks for, null for the whole file, or "invalid" for a range past the end. */
export function byteRange(header: string | undefined, size: number): { start: number; end: number } | null | "invalid" {
  const match = /^bytes=(\d*)-(\d*)$/.exec(header ?? "");
  if (!match || (!match[1] && !match[2])) return null;
  const range = match[1]
    ? { start: Number(match[1]), end: match[2] ? Math.min(Number(match[2]), size - 1) : size - 1 }
    : { start: Math.max(0, size - Number(match[2])), end: size - 1 };
  return range.start > range.end || range.start >= size ? "invalid" : range;
}

// Video players seek with Range requests, so files go out in parts when asked.
function sendFile(req: http.IncomingMessage, res: http.ServerResponse, file: string, type: string) {
  const size = fs.statSync(file).size;
  const range = byteRange(req.headers.range, size);
  if (range === "invalid") {
    res.writeHead(416, { "content-range": `bytes */${size}` }).end();
    return;
  }
  const { start, end } = range ?? { start: 0, end: size - 1 };
  res.writeHead(range ? 206 : 200, {
    "content-type": type,
    "content-length": end - start + 1,
    "accept-ranges": "bytes",
    "cache-control": "private, max-age=300",
    ...(range ? { "content-range": `bytes ${start}-${end}/${size}` } : {}),
  });
  if (req.method === "HEAD") {
    res.end();
    return;
  }
  fs.createReadStream(file, { start, end }).on("error", () => res.destroy()).pipe(res);
}

function readJson(req: http.IncomingMessage): Promise<unknown> {
  return new Promise((resolve, reject) => {
    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk: string) => (body += chunk));
    req.on("end", () => {
      try {
        resolve(JSON.parse(body));
      } catch (error) {
        reject(error);
      }
    });
    req.on("error", reject);
  });
}

/**
 * POST routes for processes started for the agents, such as their MCP servers. Any web page can POST to
 * localhost, hence the token.
 */
function handleAgentCall(apiToken: string, req: http.IncomingMessage, res: http.ServerResponse, run: (body: Record<string, unknown>) => Promise<unknown>) {
  const json = (status: number, body: unknown) =>
    res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));
  if (req.method !== "POST" || req.headers.authorization !== `Bearer ${apiToken}`) {
    json(403, { error: "Not allowed." });
    return;
  }
  readJson(req)
    .then((body) => run(body && typeof body === "object" ? (body as Record<string, unknown>) : {}))
    .then((result) => json(200, result))
    .catch((error: unknown) => json(400, { error: error instanceof Error ? error.message : String(error) }));
}

async function transcribe(app: App, { files, language }: Record<string, unknown>) {
  if (!Array.isArray(files) || !files.every((file) => typeof file === "string")) {
    throw new Error("files must be a list of file paths.");
  }
  return { text: await app.transcribe(files, typeof language === "string" ? language : undefined) };
}

async function resolveTool(app: App, { tool, input }: Record<string, unknown>) {
  if (typeof tool !== "string") throw new Error("tool must be a tool name.");
  return app.resolveTool(tool, input && typeof input === "object" && !Array.isArray(input) ? (input as Record<string, unknown>) : {});
}

async function libraryTool(app: App, { tool, input }: Record<string, unknown>) {
  if (typeof tool !== "string") throw new Error("tool must be a tool name.");
  return app.libraryTool(tool, input && typeof input === "object" && !Array.isArray(input) ? (input as Record<string, unknown>) : {});
}

export interface RemoteRoutes {
  /** Lets the user's own devices in through Tailscale Serve while remote access is on. */
  gate: () => RemoteGate | null;
  computers: () => Promise<Computer[]>;
}

const noRemote: RemoteRoutes = { gate: () => null, computers: async () => [] };

/** Signing in with Linker in the browser; see server/linker.ts. */
export type LinkerRoutes = Pick<LinkerService, "start" | "finish">;

/** Signing in to Google Drive in the browser; see server/drive/auth.ts. */
export type GoogleRoutes = Pick<GoogleAuth, "start">;

/** Linker Connect (server/connect/connect.ts): this computer's tunnel, and opening your other computers. */
export type ConnectRoutes = Pick<ConnectService, "host" | "admit" | "redeem" | "open" | "others" | "sessionOn"> & {
  sessions: Pick<Sessions, "enter" | "check" | "onRevoke" | "ticket" | "checkTicket">;
  /** The web addresses the owner put Linker-Edit's UI on and allowed at Linker (connect/webOrigins.ts). */
  webOrigins?: Pick<WebOrigins, "allows">;
  /** Asks another computer's Linker-Edit its name and version; tests replace it. */
  probe?: Probe;
};

const SESSION_COOKIE = "linker_edit_session";
/** What a page on your own web address offers when it opens /ws: this, and bearer.<its session>. */
const WS_PROTOCOL = "linker-edit";

/**
 * CORS for a page on one of your own web addresses. It sends its session as a Bearer header, never a cookie, so
 * credentials are never allowed.
 */
const CORS = {
  vary: "origin",
  "access-control-allow-headers": "authorization, content-type, range",
  "access-control-allow-methods": "GET, POST, PUT, PATCH, DELETE, OPTIONS",
  "access-control-expose-headers": "content-range, accept-ranges, content-length, www-authenticate",
  "access-control-max-age": "600",
};

/** Routes a <video>, <img> or <audio> plays from, which take a media ticket (?t=) instead of a header. */
const isMediaRoute = (urlPath: string) =>
  urlPath.startsWith("/preview/") || urlPath.startsWith("/media/") || urlPath.startsWith("/library/") || urlPath === "/api/timeline";

/** The session in Sec-WebSocket-Protocol: exactly "linker-edit" and "bearer.<session>"; null for anything else. */
export function subprotocolSession(header: string | undefined): string | null {
  const offered = (header ?? "").split(",").map((part) => part.trim());
  const bearer = offered.filter((part) => part.startsWith("bearer."));
  if (offered.length !== 2 || !offered.includes(WS_PROTOCOL) || bearer.length !== 1) return null;
  return bearer[0]!.slice("bearer.".length) || null;
}

/** The token of an Authorization: Bearer header, or null. */
function bearerToken(req: http.IncomingMessage): string | null {
  return /^Bearer (\S+)$/.exec(req.headers.authorization ?? "")?.[1] ?? null;
}

/** One cookie's value from a Cookie header. */
export function cookieValue(header: string | undefined, name: string): string | null {
  for (const part of (header ?? "").split(";")) {
    const index = part.indexOf("=");
    if (index > 0 && part.slice(0, index).trim() === name) return part.slice(index + 1).trim() || null;
  }
  return null;
}

/** A small JSON body; anything larger than limit characters is refused. */
function readSmallJson(req: http.IncomingMessage, limit = 4096): Promise<Record<string, unknown>> {
  return new Promise((resolve, reject) => {
    let body = "";
    req.setEncoding("utf8");
    req.on("data", (chunk: string) => {
      body += chunk;
      if (body.length > limit) {
        reject(new Error("Too large."));
        req.destroy();
      }
    });
    req.on("end", () => {
      try {
        const value = JSON.parse(body || "{}") as unknown;
        resolve(value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : {});
      } catch (error) {
        reject(error);
      }
    });
    req.on("error", reject);
  });
}

const escapeHtml = (text: string) =>
  text.replace(/[&<>"']/g, (char) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[char]!);

/** A small page in the browser tab that signed in, which Linker-Edit itself doesn't show. */
export function linkerPage(title: string, text: string): string {
  return `<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1">
<title>${escapeHtml(title)} · Linker-Edit</title><link rel="icon" href="/favicon.svg" type="image/svg+xml">
<style>body{margin:0;min-height:100vh;display:grid;place-items:center;font:15px/1.5 system-ui,sans-serif;background:#0b0b0c;color:#e8e8ea}
main{max-width:26rem;padding:2rem}h1{font-size:1.25rem;margin:0 0 .5rem}p{margin:0;color:#a1a1aa}
@media (prefers-color-scheme: light){body{background:#fafafa;color:#18181b}p{color:#52525b}}</style></head>
<body><main><h1>${escapeHtml(title)}</h1><p>${escapeHtml(text)}</p></main></body></html>`;
}

export function createServer(
  app: App,
  distDir: string,
  apiToken: string,
  remote: RemoteRoutes = noRemote,
  linker?: LinkerRoutes,
  connect?: ConnectRoutes,
  google?: GoogleRoutes,
): http.Server {
  const gate = remote.gate;
  const connectHost = () => connect?.host() ?? null;
  const sourceOf = (req: http.IncomingMessage) => requestSource(req.headers, gate(), connectHost());
  /** The origin of this computer's own pages at the address a request came to over Tailscale or the tunnel. */
  const ownOrigin = (source: "remote" | "connect") => `https://${source === "remote" ? gate()?.host : connectHost()}`;
  const allowsWeb = async (origin: string) => (await connect?.webOrigins?.allows(origin)) ?? false;
  /**
   * The Linker Connect session of a request through the tunnel or Tailscale: a Bearer session (the iPad app, a page on
   * your own web address), else Linker-Edit's cookie where one is taken, else a media ticket where one is taken. A
   * Bearer header that isn't a session never falls back to the others.
   */
  const sessionOf = (req: http.IncomingMessage, cookie: boolean, ticket: string | null = null) => {
    const sessions = connect?.sessions;
    if (!sessions) return null;
    const bearer = bearerToken(req);
    if (bearer !== null) return sessions.check(bearer, "native");
    return (cookie ? sessions.check(cookieValue(req.headers.cookie, SESSION_COOKIE), "cookie") : null) ?? (ticket ? sessions.checkTicket(ticket) : null);
  };
  // Your other computers, whose projects the window shows next to this one's, reached through this server.
  const machines = createMachines({
    name: os.hostname(),
    version: app.version,
    linked: async (fresh) => (connect ? connect.others(fresh) : []),
    tailscale: remote.computers,
    probe: connect?.probe ?? probeLinkerEdit,
    sessionOn: async (url, failed) => (connect ? connect.sessionOn(url, failed) : null),
  });
  /**
   * Only this computer's own window may list them and reach them through here. A <video> or <img> sends no Origin,
   * so another site's page is also told apart by Sec-Fetch-Site.
   */
  const fromOwnWindow = (req: http.IncomingMessage) =>
    sourceOf(req) === "local" &&
    isAllowedOrigin(req.headers.origin, req.headers.host) &&
    [undefined, "same-origin", "none"].includes(req.headers["sec-fetch-site"] as string | undefined);
  const server = http.createServer((req, res) => {
    handle(req, res).catch((error: unknown) => {
      console.error(error);
      if (res.headersSent) res.destroy();
      else res.writeHead(500).end();
    });
  });
  const handle = async (req: http.IncomingMessage, res: http.ServerResponse) => {
    // Also turns away pages on other sites whose name was pointed at 127.0.0.1 (DNS rebinding).
    const source = sourceOf(req);
    if (!source) {
      res.writeHead(403).end("Not allowed. Remote access is only for devices logged in to this computer's Tailscale account.");
      return;
    }
    let urlPath: string;
    let query: URLSearchParams;
    try {
      const url = new URL(req.url ?? "/", "http://localhost");
      urlPath = decodeURIComponent(url.pathname);
      query = url.searchParams;
    } catch {
      res.writeHead(400).end("Bad request");
      return;
    }
    const json = (status: number, body: unknown) =>
      res.writeHead(status, { "content-type": "application/json", "cache-control": "no-store" }).end(JSON.stringify(body));
    const page = (status: number, title: string, text: string) =>
      res
        .writeHead(status, { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", "referrer-policy": "no-referrer" })
        .end(linkerPage(title, text));
    // A page on another site, over Tailscale or through the tunnel: only your own web addresses get in, with CORS
    // and a Bearer session. This computer's own pages are same-origin and come in as before.
    const origin = req.headers.origin;
    const foreign = source !== "local" && origin !== undefined && origin !== ownOrigin(source);
    if (foreign) {
      if (!(await allowsWeb(origin))) {
        res.writeHead(403).end("Not allowed.");
        return;
      }
      res.setHeader("access-control-allow-origin", origin);
      for (const [name, value] of Object.entries(CORS)) res.setHeader(name, value);
      if (req.method === "OPTIONS") {
        // Chrome asks before a public page reaches Tailscale's 100.x addresses, which count as a local network.
        if (req.headers["access-control-request-private-network"] === "true") res.setHeader("access-control-allow-private-network", "true");
        res.writeHead(204).end();
        return;
      }
    }
    // How the desktop app on the user's other computers, and a page on your own web address, recognize this one.
    if (urlPath === "/api/hello") {
      json(200, { app: "linker-edit", name: os.hostname(), version: app.version, protocol: PROTOCOL_VERSION } satisfies Hello);
      return;
    }
    if (urlPath === "/api/machines" || urlPath.startsWith("/machines/")) {
      if (!fromOwnWindow(req)) {
        json(403, { error: "Not allowed." });
        return;
      }
      if (urlPath.startsWith("/machines/")) await machines.proxy(req, res);
      else json(200, await machines.list(query.get("fresh") === "1"));
      return;
    }
    if (source !== "local" && connect) {
      // Through the tunnel or over Tailscale, these get in without a session: they make one. Over Tailscale only the
      // owner's devices got this far.
      if (urlPath === "/api/connect/session") {
        const auth = /^Bearer (.+)$/.exec(req.headers.authorization ?? "")?.[1];
        if (req.method !== "POST" || !auth) {
          json(401, { error: "Open this computer from Linker-Edit's computer menu, signed in with Linker." });
          return;
        }
        readSmallJson(req)
          .then((body) => connect.admit(auth, typeof body.name === "string" ? body.name : ""))
          .then((result) => ("code" in result ? json(200, result) : json(result.status, { error: result.error })))
          .catch(() => json(400, { error: "Bad request." }));
        return;
      }
      // The cookie is for this computer's own pages through the tunnel only.
      if (urlPath === "/api/connect/enter" && source === "connect" && !foreign) {
        const entered = connect.sessions.enter(query.get("code") ?? "");
        if (!entered) {
          page(401, "That link ran out", `Open ${os.hostname()} again from Linker-Edit's computer menu.`);
          return;
        }
        res
          .writeHead(302, {
            location: "/",
            "cache-control": "no-store",
            "referrer-policy": "no-referrer",
            "set-cookie": `${SESSION_COOKIE}=${entered.token}; Path=/; Max-Age=${entered.maxAgeSeconds}; HttpOnly; Secure; SameSite=Lax`,
          })
          .end();
        return;
      }
      // The iPad app, or a page on your own web address, swaps its code for a session it sends as a Bearer token: in
      // the body, never in the address.
      if (urlPath === "/api/connect/native") {
        if (req.method !== "POST") {
          json(405, { error: "POST the code." });
          return;
        }
        readSmallJson(req)
          .then((body) => {
            const result = connect.redeem(typeof body.code === "string" ? body.code : "");
            if ("session" in result) json(200, result);
            else json(result.status, { error: result.error });
          })
          .catch(() => json(400, { error: "Bad request." }));
        return;
      }
      // Through the tunnel everything needs a session. Over Tailscale the owner's own devices come in as before, but
      // a page on another site, or anything showing a Bearer token, needs a session too. The cookie is only taken
      // from this computer's own pages through the tunnel, never cross-origin.
      const bearer = bearerToken(req);
      const ticket = isMediaRoute(urlPath) && (req.method === "GET" || req.method === "HEAD") ? query.get("t") : null;
      const session = sessionOf(req, source === "connect" && !foreign, ticket);
      if (!session && (source === "connect" || foreign || bearer !== null)) {
        if (bearer !== null || foreign) {
          // The iPad app and the web page sign in again when they get this.
          res.setHeader("www-authenticate", 'Bearer error="invalid_token"');
          json(401, { error: "Signed out. Sign in with Linker again." });
        } else {
          page(401, "Signed out", `Open ${os.hostname()} from Linker-Edit's computer menu, signed in with the same Linker account.`);
        }
        return;
      }
      // <video> and <img> can't send the session, so they get a ticket for it, good for an hour.
      if (urlPath === "/api/connect/media-ticket") {
        if (req.method !== "POST") {
          json(405, { error: "POST with your session." });
          return;
        }
        if (bearer === null || !session) {
          res.setHeader("www-authenticate", 'Bearer error="invalid_token"');
          json(401, { error: "Only a Bearer session gets a media ticket." });
          return;
        }
        const issued = connect.sessions.ticket(session);
        json(200, { ticket: issued.ticket, expiresAt: new Date(issued.expiresAt).toISOString() });
        return;
      }
    }
    // The desktop app opening another of your Linker Connect computers: only from this computer itself.
    if (urlPath === "/api/connect/open") {
      if (source !== "local" || req.method !== "POST" || !isAllowedOrigin(req.headers.origin, req.headers.host) || !connect) {
        json(403, { error: "Not allowed." });
        return;
      }
      readSmallJson(req)
        .then((body) => {
          if (typeof body.url !== "string") throw new Error("url must be the computer's address.");
          return connect.open(body.url);
        })
        .then((url) => json(200, { url }))
        .catch((error: unknown) => json(400, { error: error instanceof Error ? error.message : String(error) }));
      return;
    }
    if (urlPath === "/api/computers") {
      const probe = connect?.probe ?? probeLinkerEdit;
      // Your other computers linked with Linker Connect, each asked its name and version through its tunnel.
      // One linked for Tailscale only has no address at Linker; Tailscale lists it already when it's yours.
      const reachable = (computer: LinkedComputer): computer is LinkedComputer & { url: string } => computer.url !== null;
      const linked = async (): Promise<Computer[]> =>
        Promise.all(
          (connect ? await connect.others() : []).filter(reachable).map(async (computer) => {
            const origin = new URL(computer.url).origin;
            const hello = computer.online === false ? null : await probe(origin);
            return { name: hello?.name || computer.name, url: origin, version: hello?.version ?? "", via: "linker" as const, online: hello !== null };
          }),
        );
      Promise.all([remote.computers(), linked()])
        .then(([tailscale, viaLinker]) => json(200, { name: os.hostname(), version: app.version, others: [...tailscale, ...viaLinker] }))
        .catch((error: unknown) => json(500, { error: error instanceof Error ? error.message : String(error) }));
      return;
    }
    if (urlPath === "/api/linker/sign-in" || urlPath === "/api/linker/callback") {
      if (!linker) {
        page(404, "Linker sign-in is off", "This Linker-Edit can't sign in to Linker.");
        return;
      }
      // Linker sends the browser back to 127.0.0.1, which is only this computer.
      if (source !== "local") {
        page(403, "Sign in on the computer itself", "Linker can only sign in on the computer Linker-Edit runs on.");
        return;
      }
      if (urlPath === "/api/linker/sign-in") {
        linker
          .start()
          .then((location) => res.writeHead(302, { location, "cache-control": "no-store", "referrer-policy": "no-referrer" }).end())
          .catch((error: unknown) => page(502, "Couldn't reach Linker", error instanceof Error ? error.message : String(error)));
      } else {
        linker
          .finish(query)
          .then((name) => page(200, `Signed in as ${name}`, "You're signed in to Linker. Close this tab and go back to Linker-Edit."))
          .catch((error: unknown) => page(400, "Not signed in", error instanceof Error ? error.message : String(error)));
      }
      return;
    }
    if (urlPath === "/api/google/sign-in") {
      if (!google) {
        page(404, "Google Drive isn't set up", "This build of Linker-Edit can't sign in to Google.");
      } else if (source !== "local") {
        // Google sends the browser back to 127.0.0.1, which is only this computer.
        page(403, "Sign in on the computer itself", "Google can only sign in on the computer Linker-Edit runs on.");
      } else {
        google
          .start()
          .then((location) => res.writeHead(302, { location, "cache-control": "no-store", "referrer-policy": "no-referrer" }).end())
          .catch((error: unknown) => page(502, "Couldn't start signing in", error instanceof Error ? error.message : String(error)));
      }
      return;
    }
    if (urlPath === "/api/transcribe") {
      handleAgentCall(apiToken, req, res, (body) => transcribe(app, body));
      return;
    }
    if (urlPath === "/api/resolve-tool") {
      handleAgentCall(apiToken, req, res, (body) => resolveTool(app, body));
      return;
    }
    if (urlPath === "/api/library-tool") {
      handleAgentCall(apiToken, req, res, (body) => libraryTool(app, body));
      return;
    }
    const run = { "/api/study-tool": app.studyTool, "/api/brief-tool": app.briefTool, "/api/skill-tool": app.skillTool }[urlPath];
    if (run) {
      handleAgentCall(apiToken, req, res, async ({ tool, input }) => {
        if (typeof tool !== "string") throw new Error("tool must be a tool name.");
        return run(tool, input && typeof input === "object" && !Array.isArray(input) ? (input as Record<string, unknown>) : {});
      });
      return;
    }
    // Plays library files on the library page. Every request already passed the gate above: this computer, or
    // the owner's own devices over Tailscale. Only files inside library folders go out; see library/serve.ts.
    if (urlPath.startsWith("/library/")) {
      const [folderId = "", ...rest] = urlPath.slice("/library/".length).split("/");
      const file = req.method === "GET" || req.method === "HEAD" ? app.libraryFile(folderId, rest.join("/")) : null;
      if (!file) {
        res.writeHead(404, { "cache-control": "no-store" }).end();
        return;
      }
      // A file is only ever shown as media, never run as a page.
      res.setHeader("x-content-type-options", "nosniff");
      res.setHeader("content-security-policy", "default-src 'none'; sandbox");
      sendFile(req, res, file.path, file.type);
      return;
    }
    if (urlPath === "/api/timeline") {
      app
        .playableTimeline(query.get("project") || null, query.get("id") || null, query.get("commit") || null)
        .then((timeline) => (timeline ? json(200, timeline) : json(404, { error: "That timeline isn't in Resolve or in that save." })))
        .catch((error: unknown) => json(500, { error: error instanceof Error ? error.message : String(error) }));
      return;
    }
    if (urlPath === "/api/usage" || urlPath === "/api/usage/limits") {
      const usage = app.usage;
      const window = urlPath === "/api/usage" ? parseUsageWindow(query) : null;
      if (!usage) json(404, { error: "Usage isn't available here." });
      else if (urlPath === "/api/usage/limits") usage.limits().then((limits) => json(200, limits), (error: unknown) => json(500, { error: String(error) }));
      else if (!window) json(400, { error: "Ask for since and until days, or from and to times." });
      else usage.read(window, parseUsageProviders(query)).then((report) => json(200, report), (error: unknown) => json(500, { error: String(error) }));
      return;
    }
    if (urlPath.startsWith("/media/")) {
      const media = app.mediaFile(urlPath.slice("/media/".length));
      if (media) sendFile(req, res, media.path, media.type);
      else res.writeHead(404).end();
      return;
    }
    if (urlPath.startsWith("/preview/")) {
      const preview = app.previewFile(urlPath.slice("/preview/".length));
      if (preview) sendFile(req, res, preview, "video/mp4");
      else res.writeHead(404).end();
      return;
    }
    const file = staticFile(distDir, urlPath);
    if (!file) {
      res.writeHead(404).end("UI not built. Use `npm run dev`, or `npm run build` then `npm start`.");
      return;
    }
    const ext = path.extname(file);
    const headers = {
      "content-type": MIME[ext] ?? "application/octet-stream",
      "cache-control": file.endsWith(`${path.sep}index.html`) ? "no-cache" : staticCacheControl(urlPath),
      vary: foreign ? "origin, accept-encoding" : "accept-encoding",
    };
    // This computer's own window reads the files straight from disk; other devices get them gzipped.
    if (source !== "local" && COMPRESSIBLE.has(ext) && acceptsGzip(req.headers["accept-encoding"])) {
      gzippedFile(file).then(
        (data) => res.writeHead(200, { ...headers, "content-encoding": "gzip", "content-length": data.length }).end(req.method === "HEAD" ? undefined : data),
        () => res.writeHead(500).end(),
      );
      return;
    }
    res.writeHead(200, headers);
    fs.createReadStream(file).pipe(res);
  };

  /** The Linker Connect session each socket let in came with, so signing it out closes the socket. */
  const socketSessions = new WeakMap<http.IncomingMessage, string>();
  /** Whether a /ws request gets in; a session it came with is kept in socketSessions. */
  const admitSocket = async (req: http.IncomingMessage): Promise<boolean> => {
    const source = sourceOf(req);
    const origin = req.headers.origin;
    if (source === "local") return isAllowedOrigin(origin, req.headers.host);
    if (source !== "remote" && source !== "connect") return false;
    const own = ownOrigin(source);
    const admit = (session: string | null) => {
      if (session) socketSessions.set(req, session);
      return session !== null;
    };
    // A page on your own web address can't add headers to a WebSocket: its session comes as a subprotocol, and only
    // from an address the owner allowed. Over Tailscale only the owner's devices got this far.
    const offered = req.headers["sec-websocket-protocol"];
    if (offered !== undefined) {
      const session = subprotocolSession(offered);
      if (!session || !connect || origin === undefined || origin === own || !(await allowsWeb(origin))) return false;
      return admit(connect.sessions.check(session, "native"));
    }
    // A Bearer token comes from a native app, which sends no Origin, or one naming this address.
    if (bearerToken(req) !== null) return (origin === undefined || origin === own) && admit(sessionOf(req, false));
    // Over Tailscale, the owner's own pages as before; through the tunnel, this computer's own pages with the cookie.
    if (source === "remote") return origin === own;
    return origin === own && admit(sessionOf(req, true));
  };
  const verifyClient = (info: { req: http.IncomingMessage }, done: (result: boolean, code?: number) => void) => {
    admitSocket(info.req).then(
      (ok) => done(ok, 401),
      () => done(false, 500),
    );
  };
  // A page on your own web address asked for this subprotocol, and gets it.
  const handleProtocols = (protocols: Set<string>) => (protocols.has(WS_PROTOCOL) ? WS_PROTOCOL : false);
  // Messages to other devices are compressed when they offer it: a long thread's snapshot is about a megabyte of
  // JSON. This computer's own window skips the work.
  const local = new WebSocketServer({ noServer: true, verifyClient, handleProtocols });
  const fromAfar = new WebSocketServer({ noServer: true, verifyClient, handleProtocols, perMessageDeflate: { threshold: 1024 } });
  server.on("upgrade", (req: http.IncomingMessage, socket: Duplex, head: Buffer) => {
    if ((req.url ?? "/").startsWith("/machines/")) {
      if (!fromOwnWindow(req)) socket.end("HTTP/1.1 403 Forbidden\r\nConnection: close\r\n\r\n");
      else
        machines.upgrade(req, socket, head).catch((error: unknown) => {
          console.error(error);
          socket.destroy();
        });
      return;
    }
    if ((req.url ?? "/").split("?")[0] !== "/ws") {
      socket.end("HTTP/1.1 400 Bad Request\r\nConnection: close\r\n\r\n");
      return;
    }
    const wss = sourceOf(req) === "local" ? local : fromAfar;
    wss.handleUpgrade(req, socket, head, (ws) => wss.emit("connection", ws, req));
  });
  // A device signed out of Linker Connect is cut off at once, not when it next reconnects.
  const bySession = new Map<string, Set<{ close: () => void }>>();
  connect?.sessions.onRevoke((ids) => {
    for (const id of ids) for (const socket of bySession.get(id) ?? []) socket.close();
  });
  const onConnection = (socket: WebSocket, req: http.IncomingMessage) => {
    const session = socketSessions.get(req);
    if (session) {
      const sockets = bySession.get(session) ?? new Set();
      sockets.add(socket);
      bySession.set(session, sockets);
      socket.on("close", () => {
        sockets.delete(socket);
        if (sockets.size === 0) bySession.delete(session);
      });
    }
    const client = {
      send: (data: string) => socket.send(data),
      metaPatches: new URL(req.url ?? "/", "http://localhost").searchParams.get("meta") === "patch",
    };
    const disconnect = app.connect(client);
    socket.on("message", (data) => {
      try {
        app.handleMessage(client, JSON.parse(String(data)) as ClientMessage);
      } catch (error) {
        console.error("Bad client message", error);
      }
    });
    socket.on("close", disconnect);
    // Without a listener, a reset connection would crash the server.
    socket.on("error", (error) => console.error("WebSocket error", error.message));
  };
  local.on("connection", onConnection);
  fromAfar.on("connection", onConnection);

  return server;
}
