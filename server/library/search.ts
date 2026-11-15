// Finding files in the library: words in the file's name, tags and note, its kind, and a length range.
// All on this computer; nothing is sent anywhere.
import type { LibraryFile, LibraryQuery } from "../../shared/protocol.ts";

const words = (text: string) =>
  text
    .toLowerCase()
    .split(/[\s,]+/)
    .filter(Boolean);

// "Upbeat_Summer-Vibes.mp3" matches "summer vibes".
const searchable = (text: string) => text.toLowerCase().replace(/[_\-.]+/g, " ");

/** How well a file matches the words: 0 when one is missing, more when they're in its name. */
function score(file: LibraryFile, query: string[]): number {
  if (!query.length) return 1;
  const name = searchable(file.name);
  const rest = searchable([file.rel, ...file.tags, file.note].join(" "));
  let total = 0;
  for (const word of query) {
    if (name.includes(word)) total += 3;
    else if (rest.includes(word)) total += 1;
    else return 0;
  }
  return total;
}

/**
 * The files that match, best first. scopeOrder names the scopes least specific first, as the chain goes: files from
 * the most specific library come first, then starred ones, then the best matches.
 */
export function searchLibrary(files: LibraryFile[], query: LibraryQuery, scopeOrder: string[] = []): LibraryFile[] {
  const terms = words(query.query ?? "");
  const tags = (query.tags ?? []).map((tag) => tag.trim().toLowerCase()).filter(Boolean);
  const min = typeof query.minSeconds === "number" && Number.isFinite(query.minSeconds) ? query.minSeconds : null;
  const max = typeof query.maxSeconds === "number" && Number.isFinite(query.maxSeconds) ? query.maxSeconds : null;
  const rank = (file: LibraryFile) => scopeOrder.indexOf(file.scope);
  return files
    .filter((file) => !query.kind || file.kind === query.kind)
    .filter((file) => !query.starred || file.star)
    .filter((file) => tags.every((tag) => file.tags.some((own) => own.toLowerCase() === tag)))
    .filter((file) => (min === null && max === null) || (file.duration !== null && (min === null || file.duration >= min) && (max === null || file.duration <= max)))
    .map((file) => ({ file, score: score(file, terms) }))
    .filter((entry) => entry.score > 0)
    .sort(
      (a, b) =>
        rank(b.file) - rank(a.file) ||
        Number(b.file.star) - Number(a.file.star) ||
        b.score - a.score ||
        a.file.rel.localeCompare(b.file.rel, undefined, { numeric: true, sensitivity: "base" }),
    )
    .map((entry) => entry.file);
}
