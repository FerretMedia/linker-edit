// Which file GET /library/<folder id>/<path> may send. Remote access (Tailscale) reaches this server, so only a
// playable file that is really inside a library folder goes out: the path is resolved through every link first,
// and anything else is refused.
import * as fs from "node:fs";
import * as path from "node:path";
import type { LibraryFolder } from "../../shared/protocol.ts";

/**
 * What the library page plays or shows. Nothing a browser would run as a page or a script: HTML and SVG from a
 * library folder would run on this app's own address, next to its WebSocket.
 */
const PLAYABLE: Record<string, string> = {
  ".mp3": "audio/mpeg",
  ".wav": "audio/wav",
  ".m4a": "audio/mp4",
  ".aac": "audio/aac",
  ".ogg": "audio/ogg",
  ".opus": "audio/ogg",
  ".flac": "audio/flac",
  ".aif": "audio/aiff",
  ".aiff": "audio/aiff",
  ".mp4": "video/mp4",
  ".m4v": "video/mp4",
  ".mov": "video/quicktime",
  ".webm": "video/webm",
  ".png": "image/png",
  ".jpg": "image/jpeg",
  ".jpeg": "image/jpeg",
  ".gif": "image/gif",
  ".webp": "image/webp",
};

export const isPlayable = (file: string) => path.extname(file).toLowerCase() in PLAYABLE;

/** Is target inside root (not root itself)? Both already resolved; Windows compares without case. */
export function isInside(root: string, target: string): boolean {
  const relative = path.relative(root, target);
  return relative !== "" && !path.isAbsolute(relative) && relative.split(path.sep)[0] !== "..";
}

/**
 * The file to send for a folder id and a path inside that folder (/ between folders), or null. Refuses an unknown
 * folder, an absolute path or drive, "..", "." and hidden names, a file a link leads out of the folder, a missing
 * file, a folder, and any file the page doesn't play.
 */
export function libraryFile(folders: LibraryFolder[], folderId: string, rel: string): { path: string; type: string } | null {
  const folder = folders.find((candidate) => candidate.id === folderId);
  if (!folder || !rel || rel.includes("\0")) return null;
  const parts = rel.split(/[\\/]/);
  if (parts.some((part) => part === "" || part.startsWith(".") || part.includes(":"))) return null;
  const type = PLAYABLE[path.extname(rel).toLowerCase()];
  if (!type) return null;
  try {
    const root = fs.realpathSync.native(folder.path);
    const real = fs.realpathSync.native(path.join(root, ...parts));
    if (!isInside(root, real) || !fs.statSync(real).isFile()) return null;
    return { path: real, type };
  } catch {
    return null;
  }
}

/** "/library/<id>/<path>" for a file, each part encoded. */
export const libraryUrl = (folderId: string, rel: string) =>
  `/library/${encodeURIComponent(folderId)}/${rel.split("/").map(encodeURIComponent).join("/")}`;
