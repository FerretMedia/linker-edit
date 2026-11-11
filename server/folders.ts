// Lists folders on this computer for the add-project dialog's folder picker, the way T3 Code's
// "Add project" browses: the page asks for one folder at a time and filters what you type itself.
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { FolderEntry, FolderListing } from "#shared/protocol.ts";
import { PROJECT_FILE } from "./projects/store.ts";

/** More than anyone scrolls through; a folder of camera cards shouldn't send thousands of rows. */
const MAX_ENTRIES = 1000;
const ROOTS_TTL_MS = 30_000;
/** A mapped network drive that's offline can take long to answer; it's left out instead. */
const DRIVE_TIMEOUT_MS = 1500;
// Folders Windows keeps at the root of every drive, of no use for a project.
const WINDOWS_SYSTEM = new Set(["$recycle.bin", "system volume information", "$windows.~ws", "$windows.~bt", "config.msi"]);

/** ~ and ~/… as the home folder, like a shell; anything else as given. */
export function expandHome(value: string, home = os.homedir()): string {
  const trimmed = value.trim();
  if (trimmed === "~") return home;
  if (trimmed.startsWith("~/") || trimmed.startsWith("~\\")) return path.join(home, trimmed.slice(2));
  return trimmed;
}

const withSeparator = (dir: string) => (dir.endsWith(path.sep) ? dir : dir + path.sep);

async function isProject(dir: string): Promise<boolean> {
  try {
    return (await fs.promises.stat(path.join(dir, PROJECT_FILE))).isFile();
  } catch {
    return false;
  }
}

async function isFolder(entry: fs.Dirent, dir: string): Promise<boolean> {
  if (entry.isDirectory()) return true;
  // On Windows these are mostly the old junctions such as "Application Data", which can't be opened.
  if (!entry.isSymbolicLink() || process.platform === "win32") return false;
  try {
    return (await fs.promises.stat(path.join(dir, entry.name))).isDirectory();
  } catch {
    return false;
  }
}

let roots: { at: number; list: Promise<string[]> } | null = null;

function driveExists(drive: string): Promise<boolean> {
  return Promise.race([
    fs.promises.access(drive).then(
      () => true,
      () => false,
    ),
    new Promise<boolean>((resolve) => setTimeout(() => resolve(false), DRIVE_TIMEOUT_MS).unref()),
  ]);
}

/** The drives on Windows, checked at most every half minute; / elsewhere. */
export function listRoots(): Promise<string[]> {
  if (process.platform !== "win32") return Promise.resolve(["/"]);
  if (roots && Date.now() - roots.at < ROOTS_TTL_MS) return roots.list;
  const drives = [..."ABCDEFGHIJKLMNOPQRSTUVWXYZ"].map((letter) => `${letter}:\\`);
  const list = Promise.all(drives.map(driveExists)).then((exists) => drives.filter((_, index) => exists[index]));
  roots = { at: Date.now(), list };
  return list;
}

/**
 * The folders inside dir, an absolute path or one starting with ~; an empty dir is the home folder.
 * Relative paths are refused: the server's working folder means nothing to the person picking.
 */
export async function browseFolders(dir: string, home = os.homedir()): Promise<FolderListing> {
  const expanded = dir.trim() ? expandHome(dir, home) : home;
  if (!path.isAbsolute(expanded)) throw new Error(`Start the path from a drive or ~, such as ${path.join(home, "Videos")}.`);
  const resolved = withSeparator(path.resolve(expanded));
  let dirents: fs.Dirent[];
  try {
    dirents = await fs.promises.readdir(resolved, { withFileTypes: true });
  } catch (error) {
    const code = (error as NodeJS.ErrnoException).code;
    if (code === "EACCES" || code === "EPERM") dirents = [];
    // Windows says ENOENT for a file with a separator after it.
    else if (code === "ENOTDIR" || fs.statSync(resolved.slice(0, -1), { throwIfNoEntry: false })?.isFile()) {
      throw new Error(`${resolved.slice(0, -1)} is a file, not a folder.`);
    } else if (code === "ENOENT") throw new Error(`There's no folder ${resolved}`);
    else throw error;
  }
  const windows = process.platform === "win32";
  const candidates = dirents.filter((entry) => !(windows && WINDOWS_SYSTEM.has(entry.name.toLowerCase())));
  const folders = (await Promise.all(candidates.map(async (entry) => ((await isFolder(entry, resolved)) ? entry.name : null))))
    .filter((name): name is string => name !== null)
    .sort((a, b) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" }))
    .slice(0, MAX_ENTRIES);
  const entries: FolderEntry[] = await Promise.all(folders.map(async (name) => ({ name, isProject: await isProject(path.join(resolved, name)) })));
  return { dir: resolved, isProject: await isProject(resolved), entries, roots: await listRoots(), home: withSeparator(home) };
}
