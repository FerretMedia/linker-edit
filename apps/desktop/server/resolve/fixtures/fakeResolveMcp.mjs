// A stand-in for Resolve's MCP server (ResolveMCP) speaking MCP over stdio, for connection tests.
// A script's text picks what happens: "HANG" never answers, "DIE" exits, "ERROR" fails like a traceback,
// "CLOSED" answers as with Resolve closed, "SLOW" as a script that ran out of time;
// any other script answers {"result": {length, timeout}}. FAKE_NO_RUN_SCRIPT=1 leaves run_script out.
import { createInterface } from "node:readline";

/** @param {object} message */
const send = (message) => process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
/** @param {number} id @param {string} value */
const text = (id, value) => send({ id, result: { content: [{ type: "text", text: value }] } });
let calls = 0;

createInterface({ input: process.stdin }).on("line", (line) => {
  const message = JSON.parse(line);
  if (message.id === undefined) return;
  if (message.method === "initialize") {
    send({ id: message.id, result: { protocolVersion: "2024-11-05", capabilities: { tools: {} }, serverInfo: { name: "fake", version: "0" } } });
  } else if (message.method === "tools/list") {
    const tools = [{ name: "get_resolve_status" }, ...(process.env.FAKE_NO_RUN_SCRIPT ? [] : [{ name: "run_script" }])];
    send({ id: message.id, result: { tools } });
  } else if (message.params?.name === "get_resolve_status") {
    calls += 1;
    text(message.id, JSON.stringify({ running: message.params.arguments.running ?? true, calls, pid: process.pid }));
  } else if (message.params?.name === "run_script") {
    const script = message.params.arguments.script;
    if (script.includes("HANG")) return;
    if (script.includes("DIE")) process.exit(3);
    if (script.includes("ERROR")) text(message.id, JSON.stringify({ error: "Traceback (most recent call last):\n  File \"<script>\"\nRefused: Not on this timeline." }));
    else if (script.includes("CLOSED")) text(message.id, "Error: DaVinci Resolve is not running, start it with the `launch_resolve` tool.");
    else if (script.includes("SLOW")) text(message.id, JSON.stringify({ error: "Script timed out after 10s. Avoid long iterations; fetch only what you need." }));
    else text(message.id, JSON.stringify({ result: { length: script.length, timeout: message.params.arguments.timeout } }));
  }
});
