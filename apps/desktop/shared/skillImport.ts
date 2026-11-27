// What a drop of skill files may hold, and which files it leaves out. The UI checks before it reads and sends
// anything; the server checks again.

const KB = 1024;
const MB = 1024 * KB;

export const SKILL_IMPORT_LIMITS = {
  /** Every file of one drop together, zips as they are. */
  dropBytes: 8 * MB,
  /** One file of a skill. Claude's own skills keep references and scripts well under this. */
  fileBytes: 256 * KB,
  skillFiles: 200,
  skillBytes: 2 * MB,
  /** What the zips of one drop may unpack to, and how many files they may hold. */
  unpackedBytes: 32 * MB,
  zipEntries: 2_000,
} as const;

/** "8 MB", "256 KB". */
export const formatBytes = (bytes: number) => (bytes >= MB ? `${Math.round(bytes / MB)} MB` : `${Math.round(bytes / KB)} KB`);

// What Finder, Explorer and zip tools add next to the files people meant.
const JUNK = new Set(["__macosx", "desktop.ini", "thumbs.db"]);

/** The hidden file or folder a path is in, such as "color/.git" for "color/.git/config"; null when there is none. */
export function hiddenPart(path: string): string | null {
  const parts = path.split(/[\\/]/).filter(Boolean);
  const at = parts.findIndex((part) => part.startsWith(".") || JUNK.has(part.toLowerCase()));
  return at < 0 ? null : parts.slice(0, at + 1).join("/");
}
