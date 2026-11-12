// The "linker" MCP server agents get: Linker-Edit's own Resolve tools and transcription. It runs as the
// agent's child process and hands each call to the Linker-Edit server, which owns the one connection to
// Resolve, asks the user before changes, and owns the GPU queue and the transcript cache.
import * as fs from "node:fs";
import * as path from "node:path";
import { createInterface } from "node:readline";
import { CORE_TOOLS, RESOLVE_TOOLS } from "../resolve/tools.ts";
import { postApi } from "./api.ts";

const TRANSCRIBE = {
  name: "transcribe",
  description:
    "Transcribe the speech in media files on this machine, with word timings. Pass absolute file paths (a clip's file_path from list_media). Transcripts are cached, so asking again for an unchanged file returns at once. Returns each transcript with [start-end] times in seconds from the start of the file, and the path of a JSON file with per-word times.",
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
  annotations: { readOnlyHint: true },
};

const TOOLS = [
  ...RESOLVE_TOOLS.map(({ name, description, inputSchema, kind }) => ({
    name,
    description,
    inputSchema,
    annotations: { readOnlyHint: kind === "read", destructiveHint: false },
    // Claude loads this server's tools up front; these wait behind its tool search.
    ...(CORE_TOOLS.has(name) ? {} : { _meta: { "anthropic/alwaysLoad": false } }),
  })),
  TRANSCRIBE,
];

const MIME: Record<string, string> = { ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".png": "image/png" };

/** An image file the Linker-Edit server made on this computer, as MCP image content for the agent to see. */
function imageContent(file: string) {
  const mimeType = MIME[path.extname(file).toLowerCase()];
  if (!mimeType || !fs.existsSync(file)) return [];
  return [{ type: "image", data: fs.readFileSync(file).toString("base64"), mimeType }];
}

type Message = { jsonrpc: "2.0"; id?: number | string; method?: string; params?: Record<string, unknown> };

function send(message: object) {
  process.stdout.write(`${JSON.stringify({ jsonrpc: "2.0", ...message })}\n`);
}

type Reply = { text: string; isError: boolean; images?: string[] };

async function post(route: string, body: unknown): Promise<Reply> {
  try {
    const response = await postApi(route, body);
    const parsed = JSON.parse(response.text) as { text?: string; isError?: boolean; error?: string; images?: string[] };
    if (response.status !== 200) return { text: parsed.error ?? response.text, isError: true };
    return { text: parsed.text ?? "", isError: parsed.isError === true, ...(parsed.images ? { images: parsed.images } : {}) };
  } catch (error) {
    return { text: `Linker-Edit isn't reachable: ${error instanceof Error ? error.message : String(error)}`, isError: true };
  }
}

async function callTool(name: unknown, args: Record<string, unknown>): Promise<Reply> {
  if (name === TRANSCRIBE.name) {
    const files = Array.isArray(args.files) ? args.files.filter((file): file is string => typeof file === "string") : [];
    if (files.length === 0) return { text: "Pass at least one file path in files.", isError: true };
    return post("/api/transcribe", { files, language: typeof args.language === "string" ? args.language : undefined });
  }
  if (RESOLVE_TOOLS.some((tool) => tool.name === name)) return post("/api/resolve-tool", { tool: name, input: args });
  return { text: `Unknown tool ${String(name)}.`, isError: true };
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
          serverInfo: { name: "linker-edit", version: process.env.LINKER_EDIT_VERSION || "0.0.0" },
        },
      });
      break;
    case "ping":
      send({ id: message.id, result: {} });
      break;
    case "tools/list":
      send({ id: message.id, result: { tools: TOOLS } });
      break;
    case "tools/call": {
      const { text, isError, images = [] } = await callTool(message.params?.name, (message.params?.arguments as Record<string, unknown>) ?? {});
      send({ id: message.id, result: { content: [{ type: "text", text }, ...images.flatMap(imageContent)], isError } });
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
