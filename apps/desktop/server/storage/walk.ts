import * as fs from "node:fs";
import * as path from "node:path";

/** What a folder held when it was last listed. */
export interface Listing {
  mtimeMs: number;
  listedAt: number;
  dirs: string[];
  files: string[];
}

/** The listings of one walk, by folder path. */
export type Listings = Map<string, Listing>;

export interface FolderSize {
  bytes: number;
  files: number;
  /** Folders read from disk, and folders whose listing from the last walk still held. */
  listed: number;
  reused: number;
}

/**
 * Some file systems, FAT and exFAT among them, may leave a folder's mtime alone when a file is added to it, so a
 * listing is trusted this long at most.
 */
const LISTING_TRUSTED_FOR = 10 * 60_000;

export const statOf = (file: string) => {
  try {
    return fs.statSync(file, { throwIfNoEntry: false }) ?? null;
  } catch {
    return null;
  }
};

/**
 * The bytes of every file under root, as Explorer's "Size" counts them; null when root isn't a folder. A folder
 * whose mtime hasn't changed since the last walk isn't read again, so a second walk only stats. Links aren't
 * followed. Returns the listings for the next walk.
 */
export function walkFolder(root: string, last: Listings, now: number): { size: FolderSize | null; listings: Listings } {
  const listings: Listings = new Map();
  if (!statOf(root)?.isDirectory()) return { size: null, listings };
  const size: FolderSize = { bytes: 0, files: 0, listed: 0, reused: 0 };
  const pending = [root];
  for (let dir = pending.pop(); dir !== undefined; dir = pending.pop()) {
    const stat = statOf(dir);
    if (!stat) continue;
    let listing = last.get(dir);
    if (listing && listing.mtimeMs === stat.mtimeMs && now - listing.listedAt < LISTING_TRUSTED_FOR) size.reused += 1;
    else {
      let entries: fs.Dirent[];
      try {
        entries = fs.readdirSync(dir, { withFileTypes: true });
      } catch {
        continue;
      }
      listing = { mtimeMs: stat.mtimeMs, listedAt: now, dirs: [], files: [] };
      for (const entry of entries) {
        if (entry.isDirectory()) listing.dirs.push(entry.name);
        else if (entry.isFile()) listing.files.push(entry.name);
      }
      size.listed += 1;
    }
    listings.set(dir, listing);
    for (const name of listing.files) {
      const file = statOf(path.join(dir, name));
      if (!file?.isFile()) continue;
      size.bytes += file.size;
      size.files += 1;
    }
    for (const name of listing.dirs) pending.push(path.join(dir, name));
  }
  return { size, listings };
}
