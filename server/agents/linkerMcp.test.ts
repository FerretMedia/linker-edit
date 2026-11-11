import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { createInterface } from "node:readline";
import { describe, expect, it } from "vitest";
import { RESOLVE_TOOLS } from "../resolve/tools.ts";

const SERVER = path.join(import.meta.dirname, "linkerMcp.ts");

/** The "linker" MCP server in front of a stand-in Linker-Edit server that answers every call with reply. */
async function linker(reply: (route: string, body: { tool?: string; input?: unknown }) => object) {
  const seen: { route: string; auth: string | undefined; body: { tool?: string; input?: unknown } }[] = [];
  const api = http.createServer((req, res) => {
    let text = "";
    req.on("data", (chunk) => (text += chunk));
    req.on("end", () => {
      const body = JSON.parse(text) as { tool?: string; input?: unknown };
      seen.push({ route: req.url ?? "", auth: req.headers.authorization, body });
      res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(reply(req.url ?? "", body)));
    });
  });
  await new Promise<void>((resolve) => api.listen(0, "127.0.0.1", resolve));
  const port = (api.address() as AddressInfo).port;
  const child = spawn(process.execPath, [SERVER], {
    env: { ...process.env, LINKER_EDIT_API: `http://127.0.0.1:${port}`, LINKER_EDIT_TOKEN: "t0ken" },
    stdio: ["pipe", "pipe", "inherit"],
  });
  const answers = new Map<number, (message: { result: Record<string, unknown> }) => void>();
  createInterface({ input: child.stdout! }).on("line", (line) => {
    const message = JSON.parse(line) as { id: number; result: Record<string, unknown> };
    answers.get(message.id)?.(message);
  });
  let id = 0;
  const request = (method: string, params: object = {}) =>
    new Promise<Record<string, unknown>>((resolve) => {
      const n = ++id;
      answers.set(n, (message) => resolve(message.result));
      child.stdin!.write(`${JSON.stringify({ jsonrpc: "2.0", id: n, method, params })}\n`);
    });
  const close = () => {
    child.kill();
    api.close();
  };
  return { request, seen, close };
}

describe("linker MCP server", () => {
  it("lists every Resolve tool with its read-only hint, and transcribe", async () => {
    const { request, close } = await linker(() => ({}));
    try {
      const { tools } = (await request("tools/list")) as { tools: { name: string; annotations: { readOnlyHint: boolean }; _meta?: Record<string, unknown> }[] };
      expect(tools.map((tool) => tool.name)).toEqual([...RESOLVE_TOOLS.map((tool) => tool.name), "transcribe"]);
      expect(tools.find((tool) => tool.name === "view_frames")!.annotations.readOnlyHint).toBe(true);
      expect(tools.find((tool) => tool.name === "render_timeline")!.annotations.readOnlyHint).toBe(false);
      // Rarely used tools wait behind Claude's tool search.
      const deferred = (tools as { name: string; _meta?: Record<string, unknown> }[]).filter((tool) => tool._meta?.["anthropic/alwaysLoad"] === false);
      expect(deferred.map((tool) => tool.name)).toContain("set_color");
      expect(deferred.map((tool) => tool.name)).not.toContain("add_clips");
    } finally {
      close();
    }
  });

  it("passes Resolve tools to the Linker-Edit server, and sends the images it names", async () => {
    const image = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-mcp-")), "frame.jpg");
    fs.writeFileSync(image, Buffer.from([0xff, 0xd8, 0xff, 0xd9]));
    const { request, seen, close } = await linker((_route, body) =>
      body.tool === "view_frames" ? { text: '{"frames":1}', isError: false, images: [image, `${image}.gone`] } : { text: "No.", isError: true },
    );
    try {
      const frames = await request("tools/call", { name: "view_frames", arguments: { frames: [1] } });
      expect(frames).toEqual({
        content: [
          { type: "text", text: '{"frames":1}' },
          { type: "image", data: "/9j/2Q==", mimeType: "image/jpeg" },
        ],
        isError: false,
      });
      expect(await request("tools/call", { name: "create_timeline", arguments: { name: "Cut" } })).toEqual({ content: [{ type: "text", text: "No." }], isError: true });
      expect(seen).toEqual([
        { route: "/api/resolve-tool", auth: "Bearer t0ken", body: { tool: "view_frames", input: { frames: [1] } } },
        { route: "/api/resolve-tool", auth: "Bearer t0ken", body: { tool: "create_timeline", input: { name: "Cut" } } },
      ]);
    } finally {
      close();
    }
  });
});
