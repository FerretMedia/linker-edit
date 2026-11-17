// Where the library's folders are written down. The universal library and what the user notes on each file
// (tags, star, note, "used in") are in ~/LinkerEdit/library.json; a project's own folders are in its
// .linker-edit/local.json, because their paths belong to this machine and are never committed.
import { randomUUID } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { LIBRARY_KINDS, type LibraryFolder, type LibraryKind, type LibraryUse, type LocalLink } from "../../shared/protocol.ts";

/** What the user keeps on one file, by its absolute path. */
export interface FileData {
  tags: string[];
  star: boolean;
  note: string;
  usedIn: LibraryUse[];
}

interface LibraryFile {
  version: number;
  folders: LibraryFolder[];
  files: Record<string, FileData>;
}

// Bump when the file changes shape; a file with another version is kept aside rather than misread.
const VERSION = 1;
const MAX_TAGS = 30;
const MAX_TEXT = 2000;

export const isLibraryKind = (value: unknown): value is LibraryKind => LIBRARY_KINDS.includes(value as LibraryKind);

/** A folder from a file or a message, cleaned up; null when it isn't one. */
export function cleanFolder(value: unknown): LibraryFolder | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  if (typeof raw.id !== "string" || !raw.id || !isLibraryKind(raw.kind) || typeof raw.path !== "string" || !raw.path.trim()) return null;
  const folderPath = path.resolve(raw.path.trim());
  return {
    id: raw.id,
    kind: raw.kind,
    label: typeof raw.label === "string" && raw.label.trim() ? raw.label.trim().slice(0, 200) : path.basename(folderPath) || folderPath,
    path: folderPath,
    note: typeof raw.note === "string" ? raw.note.trim().slice(0, MAX_TEXT) : "",
  };
}

/** A new folder from what the user filled in; throws with what's wrong. */
export function newFolder(fields: Omit<LibraryFolder, "id">, taken: LibraryFolder[]): LibraryFolder {
  if (typeof fields.path !== "string" || !path.isAbsolute(fields.path.trim())) throw new Error("Pick the folder, starting from a drive or ~.");
  const folder = cleanFolder({ ...fields, id: randomUUID() });
  if (!folder) throw new Error("Pick a kind and a folder.");
  checkFolder(folder, taken);
  return folder;
}

/** Refuses a folder that's missing, a file, or already in this library. */
export function checkFolder(folder: LibraryFolder, others: LibraryFolder[]): void {
  let stat: fs.Stats;
  try {
    stat = fs.statSync(folder.path);
  } catch {
    throw new Error(`There is no folder ${folder.path} on this computer.`);
  }
  if (!stat.isDirectory()) throw new Error(`${folder.path} is a file, not a folder.`);
  const same = others.find((other) => other.id !== folder.id && samePath(other.path, folder.path));
  if (same) throw new Error(`${folder.path} is already in this library as "${same.label}".`);
}

/** Windows paths are the same whatever their case. */
export const pathKey = (file: string) => {
  const resolved = path.resolve(file);
  return process.platform === "win32" ? resolved.toLowerCase() : resolved;
};
export const samePath = (a: string, b: string) => pathKey(a) === pathKey(b);

function cleanData(value: unknown): FileData | null {
  if (!value || typeof value !== "object") return null;
  const raw = value as Record<string, unknown>;
  const usedIn = Array.isArray(raw.usedIn)
    ? raw.usedIn.filter(
        (use): use is LibraryUse =>
          !!use && typeof use === "object" && typeof (use as LibraryUse).projectId === "string" && typeof (use as LibraryUse).at === "string",
      )
    : [];
  return { tags: cleanTags(raw.tags), star: raw.star === true, note: typeof raw.note === "string" ? raw.note.slice(0, MAX_TEXT) : "", usedIn };
}

/** Tags as the user typed them, without duplicates or empty ones. */
export function cleanTags(value: unknown): string[] {
  if (!Array.isArray(value)) return [];
  const tags: string[] = [];
  for (const tag of value) {
    if (typeof tag !== "string") continue;
    const clean = tag.trim().replace(/\s+/g, " ").slice(0, 60);
    if (clean && !tags.some((other) => other.toLowerCase() === clean.toLowerCase())) tags.push(clean);
  }
  return tags.slice(0, MAX_TAGS);
}

export const EMPTY_DATA: FileData = { tags: [], star: false, note: "", usedIn: [] };
const isEmpty = (data: FileData) => !data.tags.length && !data.star && !data.note && !data.usedIn.length;

export interface LibraryStore {
  /** The universal library's folders. */
  folders(): LibraryFolder[];
  saveFolders(folders: LibraryFolder[]): void;
  /** What the user noted on a file; empty when nothing. */
  data(file: string): FileData;
  updateData(file: string, change: (data: FileData) => FileData): FileData;
}

/** library.json, read once and written whole on each change. A broken file is kept aside, never overwritten. */
export function fileLibraryStore(file: string, onError: (error: unknown) => void = console.error): LibraryStore {
  let state: LibraryFile = read();

  function read(): LibraryFile {
    let raw: unknown;
    try {
      raw = JSON.parse(fs.readFileSync(file, "utf8"));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code === "ENOENT") return { version: VERSION, folders: [], files: {} };
      keepAside(`can't be read (${error instanceof Error ? error.message : String(error)})`);
      return { version: VERSION, folders: [], files: {} };
    }
    const saved = raw as Partial<LibraryFile> | null;
    if (saved?.version !== VERSION) {
      keepAside("is from another Linker-Edit version");
      return { version: VERSION, folders: [], files: {} };
    }
    const folders = (Array.isArray(saved.folders) ? saved.folders : []).map(cleanFolder).filter((folder) => folder !== null);
    const files: Record<string, FileData> = {};
    for (const [key, value] of Object.entries(saved.files && typeof saved.files === "object" ? saved.files : {})) {
      const data = cleanData(value);
      if (data) files[key] = data;
    }
    return { version: VERSION, folders, files };
  }

  function keepAside(problem: string) {
    const kept = `${file}.bad-${Date.now()}`;
    try {
      fs.renameSync(file, kept);
      onError(new Error(`${file} ${problem}. It was kept as ${kept}, and the library starts empty.`));
    } catch (error) {
      onError(error);
    }
  }

  function save() {
    try {
      fs.mkdirSync(path.dirname(file), { recursive: true });
      const temp = `${file}.tmp`;
      fs.writeFileSync(temp, `${JSON.stringify(state, null, 2)}\n`);
      fs.renameSync(temp, file);
    } catch (error) {
      onError(error);
    }
  }

  return {
    folders: () => state.folders.map((folder) => ({ ...folder })),
    saveFolders(folders) {
      state = { ...state, folders: folders.map((folder) => ({ ...folder })) };
      save();
    },
    data: (target) => structuredClone(state.files[pathKey(target)] ?? EMPTY_DATA),
    updateData(target, change) {
      const key = pathKey(target);
      const next = change(structuredClone(state.files[key] ?? EMPTY_DATA));
      const files = { ...state.files };
      if (isEmpty(next)) delete files[key];
      else files[key] = next;
      state = { ...state, files };
      save();
      return structuredClone(next);
    },
  };
}

/** A library store in memory, for tests. */
export function memoryLibraryStore(folders: LibraryFolder[] = []): LibraryStore {
  let saved = folders.map((folder) => ({ ...folder }));
  const files = new Map<string, FileData>();
  return {
    folders: () => saved.map((folder) => ({ ...folder })),
    saveFolders(next) {
      saved = next.map((folder) => ({ ...folder }));
    },
    data: (target) => structuredClone(files.get(pathKey(target)) ?? EMPTY_DATA),
    updateData(target, change) {
      const next = change(structuredClone(files.get(pathKey(target)) ?? EMPTY_DATA));
      files.set(pathKey(target), next);
      return structuredClone(next);
    },
  };
}

/** A project's library folders, kept in its local.json next to the rest of this machine's link. */
export type LibraryLocal = LocalLink & { library?: unknown };

export function projectFolders(local: LocalLink): LibraryFolder[] {
  const saved = (local as LibraryLocal).library;
  return (Array.isArray(saved) ? saved : []).map(cleanFolder).filter((folder) => folder !== null);
}

export function withProjectFolders(local: LocalLink, folders: LibraryFolder[]): LocalLink {
  const next: LibraryLocal = { ...local };
  if (folders.length) next.library = folders.map((folder) => ({ ...folder }));
  else delete next.library;
  return next;
}

/** A new link for a project keeps its library folders, which aren't part of the link. */
export function keepLibrary(from: LocalLink, to: LocalLink): LocalLink {
  return withProjectFolders(to, projectFolders(from));
}
