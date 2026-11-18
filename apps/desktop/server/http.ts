import * as fs from "node:fs";
import * as http from "node:http";
import * as os from "node:os";
import * as path from "node:path";
import type { Duplex } from "node:stream";
import { promisify } from "node:util";
import * as zlib from "node:zlib";
import { WebSocketServer, type WebSocket } from "ws";
import type { ClientMessage } from "../shared/protocol.ts";
import type { App } from "./app.ts";
import type { LinkerService } from "./linker.ts";
import type { Computer } from "../shared/protocol.ts";
import type { ConnectService } from "./connect/connect.ts";
import type { Sessions } from "./connect/sessions.ts";
import { probeLinkerEdit, requestSource, type Probe, type RemoteGate } from "./remote.ts";

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

/** Linker Connect (server/connect/connect.ts): this computer's tunnel, and opening your other computers. */
export type ConnectRoutes = Pick<ConnectService, "host" | "admit" | "redeem" | "open" | "others"> & {
  sessions: Pick<Sessions, "enter" | "check" | "onRevoke">;
  /** Asks another computer's Linker-Edit its name and version; tests replace it. */
  probe?: Probe;
};

const SESSION_COOKIE = "linker_edit_session";

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
): http.Server {
  const gate = remote.gate;
  const connectHost = () => connect?.host() ?? null;
  const sourceOf = (req: http.IncomingMessage) => requestSource(req.headers, gate(), connectHost());
  /**
   * The Linker Connect session of a request through the tunnel: the iPad app's Bearer token, or else Linker-Edit's
   * cookie. Called only for "connect" requests, so a Bearer token never changes how others get in.
   */
  const sessionOf = (req: http.IncomingMessage) => {
    const bearer = bearerToken(req);
    return (
      connect?.sessions.check(bearer ?? cookieValue(req.headers.cookie, SESSION_COOKIE), bearer === null ? "cookie" : "native") ?? null
    );
  };
  const server = http.createServer((req, res) => {
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
    // How the desktop app on the user's other computers recognizes this one.
    if (urlPath === "/api/hello") {
      json(200, { app: "linker-edit", name: os.hostname(), version: app.version });
      return;
    }
    if (source === "connect" && connect) {
      // Through the Linker Connect tunnel, only these three get in without a session: they make one.
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
      if (urlPath === "/api/connect/enter") {
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
      // The iPad app swaps its code for a session it sends as a Bearer token: in the body, never in the address.
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
      if (!sessionOf(req)) {
        if (bearerToken(req) !== null) {
          // The iPad app signs in again when it gets this.
          res.setHeader("www-authenticate", 'Bearer error="invalid_token"');
          json(401, { error: "Signed out. Sign in with Linker again." });
        } else {
          page(401, "Signed out", `Open ${os.hostname()} from Linker-Edit's computer menu, signed in with the same Linker account.`);
        }
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
      const linked = async (): Promise<Computer[]> =>
        Promise.all(
          (connect ? await connect.others() : []).map(async (computer) => {
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
    if (urlPath === "/api/study-tool" || urlPath === "/api/brief-tool") {
      const run = urlPath === "/api/study-tool" ? app.studyTool : app.briefTool;
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
      vary: "accept-encoding",
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
  });

  const verifyClient = (info: { origin?: string; req: http.IncomingMessage }) => {
    const current = gate();
    switch (sourceOf(info.req)) {
      case "local":
        return isAllowedOrigin(info.origin, info.req.headers.host);
      case "remote":
        return info.origin === `https://${current!.host}`;
      case "connect": {
        // A page can't add an Authorization header to a WebSocket, so a Bearer token comes from a native app,
        // which sends no Origin, or one naming this address. A cookie comes from a page: only this computer's own.
        const own = info.origin === `https://${connectHost()}`;
        const allowed = bearerToken(info.req) === null ? own : own || info.origin === undefined;
        return allowed && sessionOf(info.req) !== null;
      }
      default:
        return false;
    }
  };
  // Messages to other devices are compressed when they offer it: a long thread's snapshot is about a megabyte of
  // JSON. This computer's own window skips the work.
  const local = new WebSocketServer({ noServer: true, verifyClient });
  const fromAfar = new WebSocketServer({ noServer: true, verifyClient, perMessageDeflate: { threshold: 1024 } });
  server.on("upgrade", (req: http.IncomingMessage, socket: Duplex, head: Buffer) => {
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
    const session = sourceOf(req) === "connect" ? sessionOf(req) : null;
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
