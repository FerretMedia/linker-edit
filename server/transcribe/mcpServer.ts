// The "linker" MCP server agents get next to Resolve's. It runs as their child process and hands
// each call to the Linker-Edit server, which owns the GPU queue and the transcript cache.
import * as http from "node:http";
import { createInterface } from "node:readline";

const api = process.env.LINKER_EDIT_API ?? "";
const token = process.env.LINKER_EDIT_TOKEN ?? "";

const TRANSCRIBE = {
  name: "transcribe",
  description:
    "Transcribe the speech in media files on this machine, with word timings. Pass absolute file paths (a clip's \"File Path\" property). Transcripts are cached, so asking again for an unchanged file returns at once. Returns each transcript with [start-end] times in seconds from the start of the file, and the path of a JSON file with per-word times.",
  inputSchema: {
    type: "object",
    properties: {
      files: { type: "array", items: { type: "string" }, description: "Absolute paths of video or audio files." },
      language: {
        type: "string",
        description: 'Spoken language as a Whisper code such as "en" or "nl". Leave out to detect it per file.',
      },
    },
    required: ["files"],
  },
};

type Message = { jsonrpc: "2.0"; id?: number | string; method?: string; params?: Record<string, unknown> };

function send(message: object) {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
}

// node:http rather than fetch: a first run downloads the engine, which outlasts fetch's 5-minute header timeout.
function post(route: string, body: unknown): Promise<{ status: number; text: string }> {
  return new Promise((resolve, reject) => {
    const request = http.request(
      `${api}${route}`,
      { method: "POST", headers: { "content-type": "application/json", authorization: `Bearer ${token}` } },
      (response) => {
        let text = "";
        response.setEncoding("utf8");
        response.on("data", (chunk: string) => (text += chunk));
        response.on("end", () => resolve({ status: response.statusCode ?? 0, text }));
      },
    );
    request.on("error", reject);
    request.end(JSON.stringify(body));
  });
}

async function callTool(name: unknown, args: Record<string, unknown>): Promise<{ text: string; isError: boolean }> {
  if (name !== TRANSCRIBE.name) return { text: `Unknown tool ${String(name)}.`, isError: true };
  const files = Array.isArray(args.files) ? args.files.filter((file): file is string => typeof file === "string") : [];
  if (files.length === 0) return { text: "Pass at least one file path in files.", isError: true };
  try {
    const response = await post("/api/transcribe", { files, language: typeof args.language === "string" ? args.language : undefined });
    const body = JSON.parse(response.text) as { text?: string; error?: string };
    return response.status === 200 ? { text: body.text ?? "", isError: false } : { text: body.error ?? response.text, isError: true };
  } catch (error) {
    return { text: `Linker-Edit isn't reachable: ${error instanceof Error ? error.message : String(error)}`, isError: true };
  }
}

async function handle(message: Message) {
  if (message.id === undefined) return;
  switch (message.method) {
    case "initialize":
      send({
        id: message.id,
        result: {
          protocolVersion: message.params?.protocolVersion ?? "2025-06-18",
          capabilities: { tools: {} },
          serverInfo: { name: "linker-edit", version: "0.1.0" },
        },
      });
      break;
    case "ping":
      send({ id: message.id, result: {} });
      break;
    case "tools/list":
      send({ id: message.id, result: { tools: [TRANSCRIBE] } });
      break;
    case "tools/call": {
      const { text, isError } = await callTool(message.params?.name, (message.params?.arguments as Record<string, unknown>) ?? {});
      send({ id: message.id, result: { content: [{ type: "text", text }], isError } });
      break;
    }
    default:
      send({ id: message.id, error: { code: -32601, message: `Method not found: ${message.method}` } });
  }
}

createInterface({ input: process.stdin }).on("line", (line) => {
  if (!line.trim()) return;
  try {
    void handle(JSON.parse(line) as Message);
  } catch {
    send({ id: null, error: { code: -32700, message: "Parse error" } });
  }
});
