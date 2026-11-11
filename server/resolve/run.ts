import * as fs from "node:fs";
import * as path from "node:path";
import { shortenText } from "../providers/shorten.ts";
import type { ResolveConnection } from "./connection.ts";
import { findResolveTool } from "./tools.ts";

/** Runs one of Linker-Edit's Resolve tools and returns its result as compact JSON, shortened when long. */
export async function runResolveTool(connection: ResolveConnection, name: string, input: Record<string, unknown>): Promise<string> {
  const tool = findResolveTool(name);
  if (!tool) throw new Error(`There is no Resolve tool called ${name}.`);
  if (name === "import_media") {
    const paths = Array.isArray(input.paths) ? input.paths : [];
    const bad = paths.filter((file) => typeof file !== "string" || !path.isAbsolute(file) || !fs.existsSync(file));
    if (paths.length === 0) throw new Error("Pass the files to import in paths.");
    if (bad.length) throw new Error(`These aren't files on this computer: ${bad.join(", ")}. Pass absolute paths.`);
  }
  const result = await connection.runScript(`tool_${name}`, input, tool.timeout ?? 30);
  const text = JSON.stringify(result);
  return shortenText(text) ?? text;
}
