import { spawn } from "node:child_process";
import { once } from "node:events";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { createInterface } from "node:readline";
import { describe, expect, it } from "vitest";

const GUARD = path.join(import.meta.dirname, "resolveGuard.ts");

// Stands in for davinci-resolve-mcp: answers every request with what it received.
const FAKE_MCP = `require("node:readline").createInterface({ input: process.stdin }).on("line", (line) => {
  const message = JSON.parse(line);
  if (message.id !== undefined) process.stdout.write(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { got: message.method } }) + "\\n");
});`;

/** Runs the guard in front of the fake MCP server, with Linker-Edit's answer given by `decide`, or no Linker-Edit at all. */
async function guard(decide: ((body: { tool: string; input: unknown }) => object) | null) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "guard-"));
  const fake = path.join(dir, "server.cjs");
  fs.writeFileSync(fake, FAKE_MCP);
  const seen: { tool: string; input: unknown; token: string | undefined }[] = [];
  const api = http.createServer((req, res) => {
    let text = "";
    req.on("data", (chunk) => (text += chunk));
    req.on("end", () => {
      const body = JSON.parse(text) as { tool: string; input: unknown };
      seen.push({ ...body, token: req.headers.authorization });
      res.writeHead(200, { "content-type": "application/json" }).end(JSON.stringify(decide!(body)));
    });
  });
  api.listen(0, "127.0.0.1");
  await once(api, "listening");
  const port = decide ? (api.address() as AddressInfo).port : 1;
  if (!decide) api.close();
  const child = spawn(process.execPath, [GUARD, process.execPath, fake], {
    env: { ...process.env, LINKER_EDIT_API: `http://127.0.0.1:${port}`, LINKER_EDIT_TOKEN: "t" },
  });
  const replies = createInterface({ input: child.stdout })[Symbol.asyncIterator]();
  return {
    seen,
    async request(message: object) {
      child.stdin.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
      return JSON.parse((await replies.next()).value as string) as unknown;
    },
    async [Symbol.asyncDispose]() {
      child.stdin.end();
      await once(child, "exit");
      api.close();
      fs.rmSync(dir, { recursive: true, force: true });
    },
  };
}

const call = (id: number, name: string, action: string) => ({ id, method: "tools/call", params: { name, arguments: { action } } });

describe("Resolve guard", () => {
  it("forwards what Linker-Edit allows and answers the rest with its reason", async () => {
    await using mcp = await guard(({ tool }) => (tool === "timeline" ? { allow: true } : { allow: false, message: "No switching." }));
    expect(await mcp.request({ id: 1, method: "tools/list" })).toEqual({ jsonrpc: "2.0", id: 1, result: { got: "tools/list" } });
    expect(await mcp.request(call(2, "timeline", "create_empty"))).toEqual({ jsonrpc: "2.0", id: 2, result: { got: "tools/call" } });
    expect(await mcp.request(call(3, "project_manager", "delete"))).toEqual({
      jsonrpc: "2.0",
      id: 3,
      result: { content: [{ type: "text", text: "No switching." }], isError: true },
    });
    expect(mcp.seen).toEqual([
      { tool: "timeline", input: { action: "create_empty" }, token: "Bearer t" },
      { tool: "project_manager", input: { action: "delete" }, token: "Bearer t" },
    ]);
  });

  it("refuses every call when Linker-Edit doesn't answer", async () => {
    await using mcp = await guard(null);
    expect(await mcp.request(call(1, "timeline", "create_empty"))).toMatchObject({
      id: 1,
      result: { isError: true, content: [{ text: expect.stringContaining("isn't reachable") }] },
    });
  });
});
