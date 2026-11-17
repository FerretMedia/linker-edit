// Full access without mistakes: the project is backed up before agents change it, agents work on "AI - "
// timelines, and an AI timeline only becomes the main one when the user says so, keeping the old one.
import * as fs from "node:fs";
import * as path from "node:path";
import { STATE_DIR } from "../projects/store.ts";
import type { ResolveConnection } from "./connection.ts";

/** Where a project's backups go, in its working folder. Kept out of the edit history. */
export const BACKUPS_DIR = "backups";
/** How many backups a project folder keeps; the oldest go first. */
export const KEEP_BACKUPS = 30;

const AI_PREFIX = "AI - ";
const pad = (n: number) => String(n).padStart(2, "0");
const day = (date: Date) => `${date.getFullYear()}-${pad(date.getMonth() + 1)}-${pad(date.getDate())}`;
// Windows file names can't hold ":", and neither need timeline names.
const fileStamp = (date: Date) => `${day(date)} ${pad(date.getHours())}-${pad(date.getMinutes())}-${pad(date.getSeconds())}`;
const nameStamp = (date: Date) => `${day(date)} ${pad(date.getHours())}.${pad(date.getMinutes())}`;

/** Exports the open Resolve project into the folder's backups/, and keeps the newest KEEP_BACKUPS. */
export async function backupProject(connection: ResolveConnection, dir: string, now = new Date()): Promise<string> {
  const backups = path.join(dir, BACKUPS_DIR);
  fs.mkdirSync(backups, { recursive: true });
  const ignore = path.join(backups, ".gitignore");
  if (!fs.existsSync(ignore)) fs.writeFileSync(ignore, "*\n");
  const { file } = await connection.runScript<{ file: string }>(
    "backup_project",
    { dir: backups.replaceAll("\\", "/"), stamp: fileStamp(now) },
    60,
  );
  if (!fs.existsSync(file)) throw new Error(`Resolve said it exported the project, but there is no ${file}.`);
  pruneBackups(backups);
  return path.relative(dir, file);
}

function pruneBackups(backups: string) {
  const files = fs
    .readdirSync(backups)
    .filter((name) => name.endsWith(".drp"))
    .map((name) => ({ file: path.join(backups, name), time: fs.statSync(path.join(backups, name)).mtimeMs }))
    .sort((a, b) => b.time - a.time);
  for (const { file } of files.slice(KEEP_BACKUPS)) fs.rmSync(file, { force: true });
}

export interface TimelineRef {
  id: string;
  name: string;
}

/** For each AI timeline, by id: the main timeline it's a new version of. */
type Origins = Record<string, TimelineRef>;

const originsFile = (dir: string) => path.join(dir, STATE_DIR, "timeline-origins.json");

export function readOrigins(dir: string): Origins {
  try {
    return JSON.parse(fs.readFileSync(originsFile(dir), "utf8")) as Origins;
  } catch {
    return {};
  }
}

function writeOrigins(dir: string, origins: Origins) {
  fs.mkdirSync(path.dirname(originsFile(dir)), { recursive: true });
  fs.writeFileSync(originsFile(dir), `${JSON.stringify(origins, null, 2)}\n`);
}

/**
 * Remembers what a new AI timeline is a version of. A version of an AI timeline replaces what that one
 * would have; one of a timeline nothing is known about replaces nothing.
 */
export function recordOrigin(dir: string, aiId: string, source: TimelineRef) {
  const origins = readOrigins(dir);
  const main = origins[source.id] ?? (source.name.startsWith(AI_PREFIX) ? undefined : source);
  if (!main) return;
  writeOrigins(dir, { ...origins, [aiId]: { id: main.id, name: main.name } });
}

export interface Applied {
  main: TimelineRef;
  previous: TimelineRef;
  /** The AI timeline's name before it took the main one's. */
  was: string;
}

/**
 * Makes an approved AI timeline the main one: the main one is renamed "<name> (before AI <date>)" and the
 * AI one takes its name. The user starts this; back the project up first.
 */
export async function applyTimeline(connection: ResolveConnection, dir: string, aiId: string, now = new Date()): Promise<Applied> {
  const origins = readOrigins(dir);
  const main = origins[aiId];
  if (!main) throw new Error("Linker-Edit doesn't know which timeline this one is a new version of.");
  const applied = await connection.runScript<Applied>("apply_timeline", { timeline: aiId, main: main.id, stamp: nameStamp(now) }, 30);
  // Other versions of the same timeline now replace the new main one.
  const next: Origins = {};
  for (const [id, origin] of Object.entries(origins)) {
    if (id === aiId) continue;
    next[id] = origin.id === main.id ? { id: aiId, name: applied.main.name } : origin;
  }
  writeOrigins(dir, next);
  return applied;
}
