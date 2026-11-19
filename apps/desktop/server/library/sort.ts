// Sorts a big assets folder (video-editing/ with Music/, SoundEffects/, VideoFX/, Images/…) into library folders:
// each folder inside it gets a kind, guessed from its name or else from its files. The user checks the guesses
// before anything is added; nothing here writes anything.
import * as fs from "node:fs";
import * as path from "node:path";
import type { LibraryFileCounts, LibraryKind, LibrarySort, LibrarySortedFolder } from "../../shared/protocol.ts";
import { AUDIO, IMAGE, LUT, VIDEO } from "./scan.ts";

/** Words in a folder's name, checked in this order: "Sound effects" before "Sound", "VideoFX" before "Video". */
const NAME_RULES: [LibraryKind, string[]][] = [
  ["luts", ["lut", "looks", "colorgrad", "grades"]],
  ["sfx", ["sfx", "soundeffect", "soundfx", "audiofx", "sounds", "foley", "whoosh", "riser", "impacts", "ambience"]],
  ["music", ["music", "songs", "tracks", "bgm", "soundtrack", "beats", "musik", "muziek", "musique"]],
  [
    "graphics",
    [
      "videofx",
      "videoeffect",
      "effect",
      "vfx",
      "gfx",
      "graphic",
      "image",
      "logo",
      "title",
      "overlay",
      "sticker",
      "icon",
      "photo",
      "picture",
      "texture",
      "background",
      "lowerthird",
      "transition",
      "element",
      "emoji",
      "meme",
      "brand",
      "afbeelding",
    ],
  ],
  ["footage", ["footage", "broll", "stock", "clips", "videos", "video", "drone", "camera", "beelden"]],
];

/** Folders deeper than this, or with more files than this, are counted only so far: it's a guess, not a scan. */
const COUNT_DEPTH = 6;
const COUNT_MAX = 3000;
const MAX_FOLDERS = 100;

const squash = (name: string) => name.toLowerCase().replace(/[^a-z0-9]/g, "");

/** A kind from a folder's name alone; null when the name doesn't say. */
export function kindFromName(name: string): LibraryKind | null {
  const word = squash(name);
  if (!word) return null;
  for (const [kind, words] of NAME_RULES) if (words.some((candidate) => word.includes(candidate))) return kind;
  return null;
}

/**
 * A kind from what a folder holds, when most of it is one kind of file. Audio says nothing either way: music
 * and sound effects are both audio, so the user picks.
 */
export function kindFromFiles(counts: LibraryFileCounts): LibraryKind | null {
  const media = counts.audio + counts.video + counts.image + counts.lut;
  if (!media) return null;
  const most = (count: number) => count / media >= 0.6;
  if (most(counts.lut)) return "luts";
  if (most(counts.image)) return "graphics";
  if (most(counts.video)) return "footage";
  return null;
}

async function subfolders(dir: string): Promise<string[]> {
  const entries = await fs.promises.readdir(dir, { withFileTypes: true });
  return entries
    .filter((entry) => entry.isDirectory() && !entry.name.startsWith(".") && !entry.name.startsWith("$"))
    .map((entry) => entry.name)
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }));
}

/** The kinds of files in a folder and the folders inside it, up to COUNT_MAX files. */
export async function countFiles(dir: string): Promise<{ counts: LibraryFileCounts; more: boolean }> {
  const counts: LibraryFileCounts = { audio: 0, video: 0, image: 0, lut: 0, other: 0 };
  let seen = 0;
  let more = false;
  async function walk(current: string, depth: number) {
    let entries: fs.Dirent[];
    try {
      entries = await fs.promises.readdir(current, { withFileTypes: true });
    } catch {
      return;
    }
    for (const entry of entries) {
      if (seen >= COUNT_MAX) {
        more = true;
        return;
      }
      if (entry.name.startsWith(".") || entry.isSymbolicLink()) continue;
      if (entry.isDirectory()) {
        if (depth < COUNT_DEPTH) await walk(path.join(current, entry.name), depth + 1);
        else more = true;
      } else if (entry.isFile()) {
        seen += 1;
        const ext = path.extname(entry.name).toLowerCase();
        if (AUDIO.has(ext)) counts.audio += 1;
        else if (VIDEO.has(ext)) counts.video += 1;
        else if (IMAGE.has(ext)) counts.image += 1;
        else if (LUT.has(ext)) counts.lut += 1;
        else counts.other += 1;
      }
    }
  }
  await walk(dir, 0);
  return { counts, more };
}

async function sortedFolder(root: string, dir: string): Promise<LibrarySortedFolder> {
  const name = path.basename(dir);
  const { counts, more } = await countFiles(dir);
  const byName = kindFromName(name);
  const byFiles = byName ? null : kindFromFiles(counts);
  return {
    name: path.relative(root, dir).split(path.sep).join("/"),
    path: dir,
    kind: byName ?? byFiles,
    reason: byName ? "name" : byFiles ? "files" : null,
    counts,
    more,
  };
}

/**
 * The folders inside root, each with a guessed kind. A folder whose name says nothing but whose folders' names
 * do, such as Audio/ with Music/ and SFX/ in it, is split into those.
 */
export async function sortFolder(root: string): Promise<LibrarySort> {
  const resolved = path.resolve(root);
  let names: string[];
  try {
    names = await subfolders(resolved);
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "ENOTDIR") throw new Error(`${resolved} is a file, not a folder.`);
    if (code === "ENOENT") throw new Error(`There is no folder ${resolved} on this computer.`);
    throw error;
  }
  const dirs: string[] = [];
  for (const name of names) {
    const dir = path.join(resolved, name);
    if (!kindFromName(name)) {
      const inner = await subfolders(dir).catch(() => []);
      if (inner.some((child) => kindFromName(child))) {
        dirs.push(...inner.map((child) => path.join(dir, child)));
        continue;
      }
    }
    dirs.push(dir);
  }
  const folders = await Promise.all(dirs.slice(0, MAX_FOLDERS).map((dir) => sortedFolder(resolved, dir)));
  return { root: resolved, rootKind: kindFromName(path.basename(resolved)), folders, more: dirs.length > MAX_FOLDERS };
}
