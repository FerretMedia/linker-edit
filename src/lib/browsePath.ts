// Path handling for the folder picker, adapted from T3 Code's project browsing
// (packages/client-runtime/src/state/projects.ts and filesystem.ts, MIT; see NOTICE).
// The path is typed as text for the server's computer, which may run another system than this page.
import type { FolderEntry } from "#shared/protocol.ts";

type Separator = "/" | "\\";

export const isWindowsDrivePath = (value: string) => /^[a-zA-Z]:([/\\]|$)/.test(value);
export const isUncPath = (value: string) => value.startsWith("\\\\");

function absoluteKind(value: string): "unix" | "windows" | null {
  if (isWindowsDrivePath(value) || isUncPath(value)) return "windows";
  if (value.startsWith("/")) return "unix";
  return null;
}

function preferredSeparator(value: string): Separator {
  const kind = absoluteKind(value);
  if (kind === "windows") return "\\";
  if (kind === "unix") return "/";
  return value.includes("\\") ? "\\" : "/";
}

export function hasTrailingSeparator(value: string): boolean {
  return (absoluteKind(value) === "unix" ? /\/$/ : /[\\/]$/).test(value);
}

function lastSeparatorIndex(value: string): number {
  if (absoluteKind(value) === "unix") return value.lastIndexOf("/");
  return Math.max(value.lastIndexOf("/"), value.lastIndexOf("\\"));
}

/** The folder part of what's typed: everything up to and including the last separator. */
export function browseDirectory(value: string): string {
  if (hasTrailingSeparator(value)) return value;
  if (value === "~") return `~${preferredSeparator(value)}`;
  const index = lastSeparatorIndex(value);
  return index < 0 ? "" : value.slice(0, index + 1);
}

/** What's typed after the last separator, which filters the folder's entries. */
export function browseFilter(value: string): string {
  if (hasTrailingSeparator(value) || value === "~") return "";
  return value.slice(lastSeparatorIndex(value) + 1);
}

/** Going into a folder: its name and a separator after the folder part. */
export function appendSegment(value: string, segment: string): string {
  return `${browseDirectory(value)}${segment}${preferredSeparator(value)}`;
}

function splitAbsolute(value: string): { root: string; separator: Separator; segments: string[] } | null {
  const split = (rest: string, separator: Separator) => rest.split(separator === "/" ? /\/+/ : /[\\/]+/).filter(Boolean);
  if (isWindowsDrivePath(value)) {
    const root = `${value.slice(0, 2)}\\`;
    return { root, separator: "\\", segments: split(value.slice(2), "\\") };
  }
  if (isUncPath(value)) {
    const [server, share, ...rest] = split(value, "\\");
    if (!server || !share) return null;
    return { root: `\\\\${server}\\${share}\\`, separator: "\\", segments: rest };
  }
  if (value.startsWith("/")) return { root: "/", separator: "/", segments: split(value.slice(1), "/") };
  return null;
}

/** The folder above a folder path, ending in a separator; null at a drive or the root. */
export function parentDirectory(value: string): string | null {
  const trimmed = trimSeparators(value);
  const absolute = splitAbsolute(trimmed);
  if (absolute) {
    if (absolute.segments.length === 0) return null;
    if (absolute.segments.length === 1) return absolute.root;
    return `${absolute.root}${absolute.segments.slice(0, -1).join(absolute.separator)}${absolute.separator}`;
  }
  const index = lastSeparatorIndex(trimmed);
  return index < 0 ? null : trimmed.slice(0, index + 1);
}

/** Without trailing separators, except for a root such as C:\ or /, which needs one. */
export function trimSeparators(value: string): string {
  const trimmed = value.trim();
  const stripped = trimmed.replace(/[\\/]+$/, "");
  if (!stripped) return trimmed ? trimmed[0]! : "";
  if (/^[a-zA-Z]:$/.test(stripped)) return `${stripped}\\`;
  return stripped;
}

/** The entries that start with the filter, ignoring case; dot-folders only once a dot is typed. */
export function filterEntries(entries: readonly FolderEntry[], filter: string): FolderEntry[] {
  const lower = filter.toLowerCase();
  const showHidden = filter.startsWith(".");
  return entries.filter((entry) => entry.name.toLowerCase().startsWith(lower) && (showHidden || !entry.name.startsWith(".")));
}

/** Shell-style Tab: one match goes into it, several extend the filter to what they share. */
export function tabCompletion(visible: readonly FolderEntry[], filter: string): { name: string; complete: boolean } | null {
  const [first, ...rest] = visible;
  if (!first) return null;
  if (rest.length === 0) return { name: first.name, complete: true };
  let prefix = [...first.name];
  for (const entry of rest) {
    const characters = [...entry.name];
    let length = 0;
    while (length < prefix.length && prefix[length]!.toLowerCase() === characters[length]?.toLowerCase()) length += 1;
    prefix = prefix.slice(0, length);
  }
  const name = prefix.join("");
  return name.length > filter.length ? { name, complete: false } : null;
}
