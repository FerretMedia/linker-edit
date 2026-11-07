import * as fs from "node:fs";
import * as http from "node:http";
import * as path from "node:path";
import { WebSocketServer } from "ws";
import type { ClientMessage } from "../shared/protocol.ts";
import type { App } from "./app.ts";

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
 * commands. Only pages served from this machine may connect; non-browser clients send no Origin.
 */
export function isAllowedOrigin(origin: string | undefined): boolean {
  if (origin === undefined) return true;
  try {
    const url = new URL(origin);
    return (url.protocol === "http:" || url.protocol === "https:") && LOOPBACK_HOSTS.has(url.hostname);
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

export function createServer(app: App, distDir: string): http.Server {
  const server = http.createServer((req, res) => {
    let urlPath: string;
    try {
      urlPath = decodeURIComponent(new URL(req.url ?? "/", "http://localhost").pathname);
    } catch {
      res.writeHead(400).end("Bad request");
      return;
    }
    if (urlPath.startsWith("/media/")) {
      const media = app.mediaFile(urlPath.slice("/media/".length));
      if (!media) {
        res.writeHead(404).end();
        return;
      }
      res.writeHead(200, { "content-type": media.type, "cache-control": "private, max-age=300" });
      fs.createReadStream(media.path).on("error", () => res.destroy()).pipe(res);
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

  const wss = new WebSocketServer({ server, path: "/ws", verifyClient: (info: { origin?: string }) => isAllowedOrigin(info.origin) });
  wss.on("connection", (socket) => {
    const disconnect = app.connect({ send: (data) => socket.send(data) });
    socket.on("message", (data) => {
      try {
        app.handleMessage(JSON.parse(String(data)) as ClientMessage);
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
