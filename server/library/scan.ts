// Reads what's in a library folder: each file's name, size and date, and with the ffmpeg Linker-Edit ships its
// duration, channels and loudness. Kept per folder in ~/LinkerEdit/cache/library/<folder id>.json; a rescan only
// measures files that are new or changed since.
import { execFile } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import type { LibraryFolder, LibraryKind } from "../../shared/protocol.ts";

export const AUDIO = new Set([".wav", ".mp3", ".aif", ".aiff", ".flac", ".m4a", ".aac", ".ogg", ".opus", ".wma", ".caf"]);
export const VIDEO = new Set([".mov", ".mp4", ".m4v", ".mxf", ".avi", ".mkv", ".webm", ".mts", ".m2ts", ".braw", ".r3d", ".mpg", ".mpeg"]);
export const IMAGE = new Set([".png", ".jpg", ".jpeg", ".tif", ".tiff", ".psd", ".exr", ".dpx", ".tga", ".bmp", ".gif", ".webp", ".heic", ".svg"]);
export const LUT = new Set([".cube", ".3dl", ".dat", ".mga", ".m3d", ".look", ".ilut", ".olut", ".csp"]);

/** Which files each kind of folder lists; "other" lists every file. */
const KIND_FILES: Record<LibraryKind, Set<string>[] | null> = {
  music: [AUDIO, VIDEO],
  sfx: [AUDIO, VIDEO],
  graphics: [IMAGE, VIDEO],
  luts: [LUT],
  footage: [VIDEO, IMAGE, AUDIO],
  other: null,
};

const MAX_FILES = 20_000;
const MAX_DEPTH = 12;
const PARALLEL = 3;

export interface ScannedFile {
  /** Inside the folder, with / between folders. */
  rel: string;
  size: number;
  mtimeMs: number;
  duration: number | null;
  channels: number | null;
  loudness: number | null;
  /** False when ffmpeg wasn't there to measure it, so the next scan tries again. */
  measured: boolean;
}

export interface FolderCache {
  version: number;
  /** The folder this was read from; a folder moved elsewhere starts over. */
  path: string;
  scannedAt: string;
  files: ScannedFile[];
  /** More files than a library folder lists. */
  truncated?: boolean;
}

const VERSION = 1;

export interface Probe {
  duration: number | null;
  channels: number | null;
  loudness: number | null;
}

/** Measures one file; loudness only when asked, because it decodes all of the audio. */
export type ProbeFile = (file: string, options: { loudness: boolean }) => Promise<Probe>;

export const cacheFile = (cacheDir: string, folderId: string) => path.join(cacheDir, `${folderId.replace(/[^\w-]/g, "_")}.json`);

export function readCache(cacheDir: string, folder: LibraryFolder): FolderCache | null {
  try {
    const cache = JSON.parse(fs.readFileSync(cacheFile(cacheDir, folder.id), "utf8")) as FolderCache;
    return cache.version === VERSION && cache.path === folder.path && Array.isArray(cache.files) ? cache : null;
  } catch {
    return null;
  }
}

function writeCache(cacheDir: string, folderId: string, cache: FolderCache) {
  fs.mkdirSync(cacheDir, { recursive: true });
  const file = cacheFile(cacheDir, folderId);
  const temp = `${file}.tmp`;
  fs.writeFileSync(temp, JSON.stringify(cache));
  fs.renameSync(temp, file);
}

export function removeCache(cacheDir: string, folderId: string) {
  fs.rmSync(cacheFile(cacheDir, folderId), { force: true });
}

export function listsFile(kind: LibraryKind, name: string): boolean {
  const sets = KIND_FILES[kind];
  return sets === null || sets.some((set) => set.has(path.extname(name).toLowerCase()));
}

/**
 * The files in a folder and the folders inside it, as paths with /. Hidden files and links are skipped: a link
 * could lead out of the folder, and the library only reaches what's inside it.
 */
export async function listFolder(root: string, kind: LibraryKind): Promise<{ files: string[]; truncated: boolean }> {
  const files: string[] = [];
  let truncated = false;
  async function walk(dir: string, rel: string, depth: number) {
    let entries: fs.Dirent[];
    try {
      entries = await fs.promises.readdir(dir, { withFileTypes: true });
    } catch (error) {
      if (depth === 0) throw error;
      return;
    }
    entries.sort((a, b) => a.name.localeCompare(b.name));
    for (const entry of entries) {
      if (files.length >= MAX_FILES) {
        truncated = true;
        return;
      }
      if (entry.name.startsWith(".") || entry.isSymbolicLink()) continue;
      const childRel = rel ? `${rel}/${entry.name}` : entry.name;
      if (entry.isDirectory()) {
        if (depth < MAX_DEPTH) await walk(path.join(dir, entry.name), childRel, depth + 1);
      } else if (entry.isFile() && listsFile(kind, entry.name)) {
        files.push(childRel);
      }
    }
  }
  await walk(root, "", 0);
  return { files, truncated };
}

/** Which files get measured, and whether for loudness: music and sound effects are, footage only for length. */
function measuring(kind: LibraryKind, rel: string): { probe: boolean; loudness: boolean } {
  const ext = path.extname(rel).toLowerCase();
  const audio = AUDIO.has(ext);
  const video = VIDEO.has(ext);
  if (!audio && !video) return { probe: false, loudness: false };
  return { probe: true, loudness: audio || kind === "music" || kind === "sfx" };
}

export interface ScanOptions {
  cacheDir: string;
  probe: ProbeFile;
  onProgress?: (done: number, total: number) => void;
  signal?: AbortSignal;
}

/** Scans a folder, measuring only files that aren't in its cache as they are now, and saves the cache. */
export async function scanFolder(folder: LibraryFolder, options: ScanOptions): Promise<FolderCache> {
  const previous = new Map((readCache(options.cacheDir, folder)?.files ?? []).map((file) => [file.rel, file]));
  const { files: names, truncated } = await listFolder(folder.path, folder.kind);
  const result: (ScannedFile | null)[] = new Array(names.length).fill(null);
  let done = 0;
  let next = 0;
  options.onProgress?.(0, names.length);

  async function scanOne(index: number) {
    const rel = names[index]!;
    let stat: fs.Stats;
    try {
      stat = await fs.promises.stat(path.join(folder.path, ...rel.split("/")));
    } catch {
      return;
    }
    const cached = previous.get(rel);
    if (cached && cached.size === stat.size && cached.mtimeMs === stat.mtimeMs && cached.measured) {
      result[index] = cached;
      return;
    }
    const plan = measuring(folder.kind, rel);
    let measured: Probe = { duration: null, channels: null, loudness: null };
    let ok = true;
    if (plan.probe) {
      try {
        measured = await options.probe(path.join(folder.path, ...rel.split("/")), { loudness: plan.loudness });
      } catch {
        ok = false;
      }
    }
    result[index] = { rel, size: stat.size, mtimeMs: stat.mtimeMs, ...measured, measured: ok };
  }

  async function worker() {
    while (next < names.length) {
      if (options.signal?.aborted) return;
      const index = next++;
      await scanOne(index);
      done += 1;
      options.onProgress?.(done, names.length);
    }
  }
  await Promise.all(Array.from({ length: PARALLEL }, worker));
  if (options.signal?.aborted) throw new Error("The scan was stopped.");

  const cache: FolderCache = {
    version: VERSION,
    path: folder.path,
    scannedAt: new Date().toISOString(),
    files: result.filter((file) => file !== null),
    ...(truncated ? { truncated } : {}),
  };
  writeCache(options.cacheDir, folder.id, cache);
  return cache;
}

const CHANNEL_LAYOUTS: Record<string, number> = { mono: 1, stereo: 2, "2.1": 3, quad: 4, "4.0": 4, "5.0": 5, "5.1": 6, "6.1": 7, "7.1": 8 };

/** Duration, channels and loudness from ffmpeg's log; null for what it doesn't say. */
export function parseProbe(log: string): Probe {
  const time = /Duration: (\d+):(\d+):([\d.]+)/.exec(log);
  const duration = time ? Number(time[1]) * 3600 + Number(time[2]) * 60 + Number(time[3]) : null;
  // The input's first audio stream: "Audio: pcm_s16le, 48000 Hz, stereo, s16" or "…, 2 channels, …".
  const input = log.split(/\nOutput #0|\nStream mapping:/)[0] ?? log;
  const audio = /Stream #\d+:\d+[^:\n]*: Audio: [^\n]*?, \d+ Hz, ([^,\n]+)/.exec(input);
  let channels: number | null = null;
  if (audio) {
    const layout = audio[1]!.trim().replace(/\(.*\)$/, "");
    const count = /^(\d+) channels?$/.exec(layout);
    channels = count ? Number(count[1]) : (CHANNEL_LAYOUTS[layout] ?? null);
  }
  const integrated = /Integrated loudness:\s*\n\s*I:\s*(-?[\d.]+) LUFS/.exec(log);
  const loudness = integrated ? Number(integrated[1]) : null;
  return {
    duration: duration !== null && Number.isFinite(duration) && duration > 0 ? Math.round(duration * 1000) / 1000 : null,
    channels,
    loudness: loudness !== null && Number.isFinite(loudness) ? loudness : null,
  };
}

/** Measures files with ffmpeg: its log has the duration and channels, and ebur128 adds the loudness. */
export function ffmpegProbe(ffmpeg: () => string): ProbeFile {
  return (file, { loudness }) =>
    new Promise((resolve, reject) => {
      let command: string;
      try {
        command = ffmpeg();
      } catch (error) {
        reject(error);
        return;
      }
      // Without an output ffmpeg stops after reading the header, which is all the length and channels need.
      const args = loudness
        ? ["-hide_banner", "-nostats", "-i", file, "-map", "0:a:0", "-af", "ebur128=framelog=verbose", "-f", "null", "-"]
        : ["-hide_banner", "-i", file];
      // A file ffmpeg can't read gets nothing measured, and isn't tried again until it changes.
      execFile(command, args, { windowsHide: true, timeout: 5 * 60_000, maxBuffer: 16 * 1024 * 1024 }, (error, _stdout, stderr) => {
        if ((error as NodeJS.ErrnoException | null)?.code === "ENOENT") reject(error);
        else resolve(parseProbe(String(stderr)));
      });
    });
}
