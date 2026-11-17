// The library as a whole: its folders' scans, what the user noted on each file, and finding files across a chain
// of libraries. Every lookup takes the chain (universal first, the project last), so more libraries in between
// (workspaces, series) only add links to it.
import * as path from "node:path";
import type { LibraryFile, LibraryFolder, LibraryFolderStatus, LibraryQuery, LibraryScope, LibraryUse } from "../../shared/protocol.ts";
import { pathKey, type FileData, type LibraryStore } from "./folders.ts";
import { readCache, removeCache, scanFolder, type FolderCache, type ProbeFile } from "./scan.ts";
import { searchLibrary } from "./search.ts";
import { isInside } from "./serve.ts";

/** One library in a chain: which one, its name, and its folders. */
export interface LibraryLink {
  scope: LibraryScope;
  label: string;
  folders: LibraryFolder[];
}

export interface LibraryOptions {
  store: LibraryStore;
  cacheDir: string;
  probe: ProbeFile;
  onError?: (error: unknown) => void;
}

export type LibraryService = ReturnType<typeof createLibrary>;

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
// Progress is sent this often at most while a scan runs.
const PROGRESS_MS = 400;

export function createLibrary(options: LibraryOptions) {
  const { store, cacheDir } = options;
  const onError = options.onError ?? console.error;
  const caches = new Map<string, FolderCache | null>();
  const scanning = new Map<string, { done: number; total: number; controller: AbortController }>();
  const problems = new Map<string, string>();
  const listeners = new Set<() => void>();
  // One folder is scanned at a time, so a library of big folders doesn't run ffmpeg dozens of times at once.
  let queue: Promise<void> = Promise.resolve();
  const queued = new Map<string, { path: string; job: Promise<void>; cancelled: boolean }>();
  let progressTimer: ReturnType<typeof setTimeout> | null = null;

  function changed() {
    if (progressTimer) {
      clearTimeout(progressTimer);
      progressTimer = null;
    }
    for (const listener of listeners) listener();
  }

  function progressed() {
    progressTimer ??= setTimeout(() => {
      progressTimer = null;
      for (const listener of listeners) listener();
    }, PROGRESS_MS);
  }

  function cacheOf(folder: LibraryFolder): FolderCache | null {
    if (!caches.has(folder.id)) caches.set(folder.id, readCache(cacheDir, folder));
    const cache = caches.get(folder.id) ?? null;
    return cache && cache.path === folder.path ? cache : null;
  }

  function status(folder: LibraryFolder): LibraryFolderStatus {
    const cache = cacheOf(folder);
    const scan = scanning.get(folder.id);
    return {
      ...folder,
      files: cache?.files.length ?? 0,
      scanning: scan ? { done: scan.done, total: scan.total } : null,
      scannedAt: cache?.scannedAt ?? null,
      problem: problems.get(folder.id) ?? (cache?.truncated ? "Only the first 20,000 files are listed. Add the folders inside it instead." : null),
    };
  }

  function fileOf(link: LibraryLink, folder: LibraryFolder, scanned: FolderCache["files"][number], data: FileData): LibraryFile {
    const file = path.join(folder.path, ...scanned.rel.split("/"));
    return {
      folderId: folder.id,
      scope: link.scope.kind,
      kind: folder.kind,
      rel: scanned.rel,
      name: scanned.rel.split("/").at(-1)!,
      path: file,
      size: scanned.size,
      modified: new Date(scanned.mtimeMs).toISOString(),
      duration: scanned.duration,
      channels: scanned.channels,
      loudness: scanned.loudness,
      tags: data.tags,
      star: data.star,
      note: data.note,
      usedIn: data.usedIn,
    };
  }

  /** Every file in a chain of libraries, with what the user noted on it. */
  function files(chain: LibraryLink[]): LibraryFile[] {
    return chain.flatMap((link) =>
      link.folders.flatMap((folder) =>
        (cacheOf(folder)?.files ?? []).map((scanned) => fileOf(link, folder, scanned, store.data(path.join(folder.path, ...scanned.rel.split("/"))))),
      ),
    );
  }

  function scan(folder: LibraryFolder): Promise<void> {
    const pending = queued.get(folder.id);
    if (pending && !pending.cancelled && pending.path === folder.path) return pending.job;
    const entry = { path: folder.path, job: Promise.resolve(), cancelled: false };
    entry.job = queue.then(async () => {
      if (entry.cancelled) return;
      const controller = new AbortController();
      scanning.set(folder.id, { done: 0, total: 0, controller });
      problems.delete(folder.id);
      changed();
      try {
        const cache = await scanFolder(folder, {
          cacheDir,
          probe: options.probe,
          signal: controller.signal,
          onProgress: (done, total) => {
            const scan = scanning.get(folder.id);
            if (scan) Object.assign(scan, { done, total });
            progressed();
          },
        });
        caches.set(folder.id, cache);
      } catch (error) {
        if (!controller.signal.aborted) {
          const code = (error as NodeJS.ErrnoException).code;
          problems.set(folder.id, code === "ENOENT" ? `${folder.path} isn't there. Is the drive connected?` : errorText(error));
        }
      } finally {
        scanning.delete(folder.id);
        if (queued.get(folder.id) === entry) queued.delete(folder.id);
        changed();
      }
    });
    queue = entry.job.catch(onError);
    queued.set(folder.id, entry);
    return entry.job;
  }

  /** The library file at an absolute path in this chain, most specific library first; null when it isn't one. */
  function find(chain: LibraryLink[], file: string): LibraryFile | null {
    const key = pathKey(file);
    for (const link of [...chain].reverse()) {
      for (const folder of link.folders) {
        if (!isInside(pathKey(folder.path), key)) continue;
        const scanned = cacheOf(folder)?.files.find((entry) => pathKey(path.join(folder.path, ...entry.rel.split("/"))) === key);
        if (scanned) return fileOf(link, folder, scanned, store.data(file));
      }
    }
    return null;
  }

  return {
    onChange(listener: () => void) {
      listeners.add(listener);
    },
    /** The universal library's folders. */
    folders: () => store.folders(),
    saveFolders: (folders: LibraryFolder[]) => store.saveFolders(folders),
    status,
    files,
    find,
    /** The files that match, from the most specific library first. */
    search: (chain: LibraryLink[], query: LibraryQuery) => searchLibrary(files(chain), query, chain.map((link) => link.scope.kind)),
    /** Scans a folder after the ones already waiting; a folder already waiting isn't queued twice. */
    scan,
    /** Rescans every folder, measuring only what changed: at start, so files added meanwhile show up. */
    scanAll: (folders: LibraryFolder[]) => Promise.all(folders.map(scan)).then(() => {}),
    /** Forgets a folder that left every library: its scan stops and its cache goes. Its files stay. */
    forget(folder: LibraryFolder) {
      const pending = queued.get(folder.id);
      if (pending) pending.cancelled = true;
      scanning.get(folder.id)?.controller.abort();
      caches.delete(folder.id);
      problems.delete(folder.id);
      try {
        removeCache(cacheDir, folder.id);
      } catch (error) {
        onError(error);
      }
      changed();
    },
    updateFile(file: string, change: Partial<Pick<FileData, "tags" | "star" | "note">>) {
      store.updateData(file, (data) => ({ ...data, ...change }));
      changed();
    },
    /** Notes that an agent brought a file into a project's timeline; the same project and timeline once. */
    recordUse(file: string, use: LibraryUse) {
      store.updateData(file, (data) => ({
        ...data,
        usedIn: [...data.usedIn.filter((other) => !(other.projectId === use.projectId && other.timeline === use.timeline)), use],
      }));
      changed();
    },
  };
}
