// The library's tools on the agents' "linker" MCP server: finding files in the library, and bringing them into
// the open Resolve project through import_media, into a "Library/<Kind>" bin.
import * as path from "node:path";
import { LIBRARY_KINDS, type LibraryFile, type LibraryKind, type LibraryQuery, type LibraryUse } from "../../shared/protocol.ts";
import { pathKey } from "./folders.ts";

export const LIBRARY_SEARCH = "library_search";
export const LIBRARY_IMPORT = "library_import";

export const LIBRARY_TOOLS = [
  {
    name: LIBRARY_SEARCH,
    description:
      "Find music, sound effects, graphics, LUTs and footage in the user's library: folders of files they reuse. Searches file names, tags and notes; filter by kind, tags and length. Results come from the project's own library first, then the universal one, starred files first. Each has its absolute path, length in seconds, channels, loudness (LUFS), tags, notes, the folder's note (licensing: respect it) and where it was used before.",
    inputSchema: {
      type: "object",
      properties: {
        kind: { type: "string", enum: [...LIBRARY_KINDS], description: "music, sfx (sound effects), graphics, luts, footage or other." },
        query: { type: "string", description: 'Words to find, such as "upbeat" or "whoosh". Leave out to list everything of a kind.' },
        tags: { type: "array", items: { type: "string" }, description: "Only files with all of these tags." },
        min_seconds: { type: "number" },
        max_seconds: { type: "number" },
        limit: { type: "integer", description: "At most this many results. Defaults to 30." },
      },
    },
    annotations: { readOnlyHint: true },
  },
  {
    name: LIBRARY_IMPORT,
    description:
      'Bring library files (paths from library_search) into the open Resolve project\'s media pool, into a "Library/<Kind>" bin such as "Library/Music". A file already in the media pool is reused, not imported twice. Returns each file\'s media pool id for add_clips. Notes the project and timeline it was used in. LUTs aren\'t media: pass their path to set_color.',
    inputSchema: {
      type: "object",
      properties: {
        files: { type: "array", items: { type: "string" }, description: "Absolute paths, as library_search returns them." },
        bin: { type: "string", description: 'Another bin to import into, such as "Library/Music/Intro". Defaults to "Library/<Kind>".' },
        timeline: { type: "string", description: "The timeline you'll use them on, for the file's \"used in\". Defaults to the one open in Resolve." },
      },
      required: ["files"],
    },
    annotations: { readOnlyHint: false, destructiveHint: false },
  },
];

export const isLibraryTool = (name: string) => name === LIBRARY_SEARCH || name === LIBRARY_IMPORT;

export const KIND_BIN: Record<LibraryKind, string> = {
  music: "Music",
  sfx: "Sound effects",
  graphics: "Graphics",
  luts: "LUTs",
  footage: "Footage",
  other: "Other",
};

const num = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : undefined);
const strings = (value: unknown) => (Array.isArray(value) ? value.filter((item): item is string => typeof item === "string" && item.trim() !== "") : []);

export function queryFromInput(input: Record<string, unknown>): LibraryQuery {
  const kind = LIBRARY_KINDS.find((candidate) => candidate === input.kind);
  const query: LibraryQuery = {};
  if (kind) query.kind = kind;
  if (typeof input.query === "string" && input.query.trim()) query.query = input.query;
  const tags = strings(input.tags);
  if (tags.length) query.tags = tags;
  const min = num(input.min_seconds);
  const max = num(input.max_seconds);
  if (min !== undefined) query.minSeconds = min;
  if (max !== undefined) query.maxSeconds = max;
  return query;
}

const round = (value: number, digits: number) => Math.round(value * 10 ** digits) / 10 ** digits;

/** A result as the agent reads it: only what's there, to keep the answer short. */
export function describeFile(file: LibraryFile, folder: { label: string; note: string }) {
  return {
    path: file.path,
    kind: file.kind,
    library: file.scope,
    folder: folder.label,
    ...(folder.note ? { folder_note: folder.note } : {}),
    ...(file.duration !== null ? { seconds: round(file.duration, 2) } : {}),
    ...(file.channels !== null ? { channels: file.channels } : {}),
    ...(file.loudness !== null ? { loudness_lufs: round(file.loudness, 1) } : {}),
    ...(file.star ? { star: true } : {}),
    ...(file.tags.length ? { tags: file.tags } : {}),
    ...(file.note ? { note: file.note } : {}),
    ...(file.usedIn.length
      ? { used_in: file.usedIn.map((use) => (use.timeline ? `${use.project} · ${use.timeline}` : use.project)) }
      : {}),
  };
}

export interface ImportContext {
  /** The library file at a path in the turn's project's chain, or null. */
  find: (file: string) => LibraryFile | null;
  /** Runs one of Linker-Edit's Resolve tools for the turn, returning its text; throws with what went wrong. */
  resolve: (tool: string, input: Record<string, unknown>) => Promise<string>;
  /** Waits for the user's OK in supervised mode and backs the project up; a refusal's text, or null to go on. */
  allowChange: (input: Record<string, unknown>) => Promise<string | null>;
  recordUse: (file: string, use: Omit<LibraryUse, "at">) => void;
  project: { id: string; name: string };
  /** The timeline open in Resolve, for "used in" when the agent names none. */
  currentTimeline: string | null;
}

interface PoolClip {
  id: string;
  name: string;
  bin?: string;
  file_path?: string;
}

/** A tool's JSON answer, which Linker-Edit may have shortened with a note after it. */
function parseResult<T>(text: string): T {
  const json = text.split("\nLinker-Edit shortened")[0]!;
  return JSON.parse(json) as T;
}

/** The media pool clip of a file already in the project, found by its file path. */
async function findInPool(context: ImportContext, file: string): Promise<PoolClip | null> {
  const result = parseResult<{ clips?: PoolClip[] }>(await context.resolve("list_media", { search: path.basename(file), limit: 500 }));
  return (result.clips ?? []).find((clip) => typeof clip.file_path === "string" && clip.file_path && pathKey(clip.file_path) === pathKey(file)) ?? null;
}

export async function libraryImport(input: Record<string, unknown>, context: ImportContext) {
  const asked = strings(input.files);
  if (asked.length === 0) throw new Error("Pass the files to import in files: absolute paths from library_search.");
  const bin = typeof input.bin === "string" && input.bin.trim() ? input.bin.trim() : null;
  const timeline = typeof input.timeline === "string" && input.timeline.trim() ? input.timeline.trim() : context.currentTimeline;

  const notInLibrary: string[] = [];
  const luts: string[] = [];
  const media: LibraryFile[] = [];
  for (const file of asked) {
    const found = path.isAbsolute(file) ? context.find(file) : null;
    if (!found) notInLibrary.push(file);
    else if (found.kind === "luts") luts.push(found.path);
    else if (!media.some((other) => pathKey(other.path) === pathKey(found.path))) media.push(found);
  }
  if (media.length === 0 && luts.length === 0) {
    throw new Error(`These aren't files in this project's library: ${notInLibrary.join(", ")}. Find them with library_search, or use import_media for other files.`);
  }

  const done: { path: string; media_id: string; name: string; bin: string | null; reused: boolean }[] = [];
  const missing: LibraryFile[] = [];
  for (const file of media) {
    const clip = await findInPool(context, file.path);
    if (clip) done.push({ path: file.path, media_id: clip.id, name: clip.name, bin: clip.bin ?? null, reused: true });
    else missing.push(file);
  }

  if (missing.length) {
    const byBin = new Map<string, LibraryFile[]>();
    for (const file of missing) {
      const target = bin ?? `Library/${KIND_BIN[file.kind]}`;
      byBin.set(target, [...(byBin.get(target) ?? []), file]);
    }
    const refusal = await context.allowChange({ files: missing.map((file) => file.path), bins: [...byBin.keys()] });
    if (refusal) throw new Error(refusal);
    for (const [target, files] of byBin) {
      const result = parseResult<{ bin?: string; imported?: PoolClip[] }>(
        await context.resolve("import_media", { paths: files.map((file) => file.path), bin: target }),
      );
      for (const file of files) {
        // Resolve names a clip after its file, and some versions leave its path empty right after an import.
        const imported = result.imported ?? [];
        const clip =
          imported.find((item) => typeof item.file_path === "string" && item.file_path && pathKey(item.file_path) === pathKey(file.path)) ??
          imported.find((item) => !item.file_path && item.name === path.basename(file.path));
        if (clip) done.push({ path: file.path, media_id: clip.id, name: clip.name, bin: target, reused: false });
      }
    }
  }

  for (const entry of done) context.recordUse(entry.path, { projectId: context.project.id, project: context.project.name, timeline });
  for (const lut of luts) context.recordUse(lut, { projectId: context.project.id, project: context.project.name, timeline });

  const failed = missing.filter((file) => !done.some((entry) => pathKey(entry.path) === pathKey(file.path))).map((file) => file.path);
  return {
    files: done,
    ...(luts.length ? { luts, luts_note: "LUTs don't go in the media pool. Apply one with set_color's lut, using its path." } : {}),
    ...(failed.length ? { not_imported: failed, note: "Resolve didn't import these. Are they media Resolve can read?" } : {}),
    ...(notInLibrary.length ? { not_in_library: notInLibrary } : {}),
  };
}
