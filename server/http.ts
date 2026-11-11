import * as fs from "node:fs";
import * as http from "node:http";
import * as os from "node:os";
import * as path from "node:path";
import { WebSocketServer } from "ws";
import type { ClientMessage } from "../shared/protocol.ts";
import type { App } from "./app.ts";
import type { Computer } from "../shared/protocol.ts";
import { requestSource, type RemoteGate } from "./remote.ts";

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

export interface RemoteRoutes {
  /** Lets the user's own devices in through Tailscale Serve while remote access is on. */
  gate: () => RemoteGate | null;
  computers: () => Promise<Computer[]>;
}

const noRemote: RemoteRoutes = { gate: () => null, computers: async () => [] };

export function createServer(app: App, distDir: string, apiToken: string, remote: RemoteRoutes = noRemote): http.Server {
  const gate = remote.gate;
  const server = http.createServer((req, res) => {
    // Also turns away pages on other sites whose name was pointed at 127.0.0.1 (DNS rebinding).
    if (!requestSource(req.headers, gate())) {
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
    // How the desktop app on the user's other computers recognizes this one.
    if (urlPath === "/api/hello") {
      json(200, { app: "linker-edit", name: os.hostname(), version: app.version });
      return;
    }
    if (urlPath === "/api/computers") {
      remote
        .computers()
        .then((others) => json(200, { name: os.hostname(), others }))
        .catch((error: unknown) => json(500, { error: error instanceof Error ? error.message : String(error) }));
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
    res.writeHead(200, { "content-type": MIME[path.extname(file)] ?? "application/octet-stream" });
    fs.createReadStream(file).pipe(res);
  });

  const wss = new WebSocketServer({
    server,
    path: "/ws",
    verifyClient: (info: { origin?: string; req: http.IncomingMessage }) => {
      const current = gate();
      switch (requestSource(info.req.headers, current)) {
        case "local":
          return isAllowedOrigin(info.origin, info.req.headers.host);
        case "remote":
          return info.origin === `https://${current!.host}`;
        default:
          return false;
      }
    },
  });
  wss.on("connection", (socket) => {
    const client = { send: (data: string) => socket.send(data) };
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
  });

  return server;
}
