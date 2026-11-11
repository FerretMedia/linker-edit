import * as fs from "node:fs";
import * as path from "node:path";
import { APP_ROOT } from "./config.ts";

const pkg = JSON.parse(fs.readFileSync(path.join(APP_ROOT, "package.json"), "utf8")) as { name: string; version: string };

/** A checkout says 0.0.0; releases are YYYY.MDD.N, stamped when they're published. People see the release name
 * instead, 2026.11.07: see shared/release-name.ts. */
export const VERSION = pkg.version;

export function isNewer(candidate: string, current: string): boolean {
  const a = candidate.split(".").map(Number);
  const b = current.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    const diff = (a[i] ?? 0) - (b[i] ?? 0);
    if (diff !== 0) return diff > 0;
  }
  return false;
}

/** The latest version on npm, or null when the registry can't be reached. */
export async function latestVersion(): Promise<string | null> {
  try {
    const response = await fetch(`https://registry.npmjs.org/${pkg.name}/latest`, { signal: AbortSignal.timeout(10_000) });
    if (!response.ok) return null;
    return ((await response.json()) as { version?: string }).version ?? null;
  } catch {
    return null;
  }
}
