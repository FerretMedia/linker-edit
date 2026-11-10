// Sits between Codex and davinci-resolve-mcp: `node resolveGuard.ts <python> <server.py>`. Codex has no
// per-call hook, so each Resolve tool call is first put to the Linker-Edit server, which denies project
// switches and, in "ask" mode, waits for the user. Everything else passes through unchanged.
import { spawn } from "node:child_process";
import { createInterface } from "node:readline";
import { postApi } from "./api.ts";

type Message = { id?: number | string; method?: string; params?: { name?: unknown; arguments?: unknown } };

/** The server's answer for one call; anything but a clear yes is a no. */
async function decide(tool: string, input: unknown): Promise<string | null> {
  try {
    const response = await postApi("/api/resolve-call", { tool, input });
    const body = JSON.parse(response.text) as { allow?: boolean; message?: string; error?: string };
    if (response.status === 200 && body.allow === true) return null;
    return body.message ?? body.error ?? `Linker-Edit refused this call (${response.status}).`;
  } catch (error) {
    return `Linker-Edit isn't reachable, so Resolve calls are refused: ${error instanceof Error ? error.message : String(error)}`;
  }
}

const [command, ...args] = process.argv.slice(2);
if (!command) {
  console.error("Usage: resolveGuard <python> <server.py> [args…]");
  process.exit(2);
}

const env = { ...process.env };
delete env.LINKER_EDIT_API;
delete env.LINKER_EDIT_TOKEN;
const child = spawn(command, args, { stdio: ["pipe", "pipe", "inherit"], env, windowsHide: true });

// Whole lines only, so a refusal written here never lands inside one of the server's messages.
const write = (line: string) => process.stdout.write(`${line}\n`);
createInterface({ input: child.stdout }).on("line", write);
child.on("exit", (code) => process.exit(code ?? 1));
child.on("error", (error) => {
  console.error(`Couldn't start the Resolve MCP server: ${error.message}`);
  process.exit(1);
});

createInterface({ input: process.stdin })
  .on("line", (line) => {
    let message: Message;
    try {
      message = JSON.parse(line) as Message;
    } catch {
      child.stdin.write(`${line}\n`);
      return;
    }
    if (message.method !== "tools/call" || message.id === undefined) {
      child.stdin.write(`${line}\n`);
      return;
    }
    void decide(String(message.params?.name), message.params?.arguments ?? {}).then((refusal) => {
      if (refusal === null) child.stdin.write(`${line}\n`);
      else write(JSON.stringify({ jsonrpc: "2.0", id: message.id, result: { content: [{ type: "text", text: refusal }], isError: true } }));
    });
  })
  .on("close", () => child.stdin.end());
