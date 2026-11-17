import type { LibraryKind, LibraryScope } from "#shared/protocol.ts";

/** The library page's tabs. Workspaces and series will add theirs. */
export type LibraryTab = "universal" | "project";

/** The library page lives in the address's hash, like the settings page: "#library" or "#library/project". */
export function libraryTabFromHash(hash: string): LibraryTab | null {
  const match = /^#library(?:\/([\w-]+))?$/.exec(hash);
  if (!match) return null;
  return match[1] === "project" ? "project" : "universal";
}

export const libraryHash = (tab: LibraryTab) => (tab === "universal" ? "#library" : `#library/${tab}`);

export function scopeOf(tab: LibraryTab, projectId: string | null): LibraryScope | null {
  if (tab === "universal") return { kind: "universal" };
  return projectId ? { kind: "project", projectId } : null;
}

export const KIND_LABEL: Record<LibraryKind, string> = {
  music: "Music",
  sfx: "Sound effects",
  graphics: "Graphics",
  luts: "LUTs",
  footage: "Footage",
  other: "Other",
};

/** What the browser plays (with an audio element) or shows as a picture, as the server's library route serves them. */
const PLAYS = new Set([".mp3", ".wav", ".m4a", ".aac", ".ogg", ".opus", ".flac", ".aif", ".aiff", ".mp4", ".m4v", ".mov", ".webm"]);
const SHOWS = new Set([".png", ".jpg", ".jpeg", ".gif", ".webp"]);
const extension = (name: string) => /\.[^.]+$/.exec(name)?.[0].toLowerCase() ?? "";
export const playable = (name: string) => PLAYS.has(extension(name));
export const showable = (name: string) => SHOWS.has(extension(name));

/** Where a library file plays from on this server. */
export const fileUrl = (folderId: string, rel: string) =>
  `/library/${encodeURIComponent(folderId)}/${rel.split("/").map(encodeURIComponent).join("/")}`;

/** 3:07, or 1:02:03 for an hour or more; under 10 seconds with a tenth, like 1.2s. */
export function formatDuration(seconds: number): string {
  if (seconds < 10) return `${(Math.round(seconds * 10) / 10).toString()}s`;
  const total = Math.round(seconds);
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = String(total % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

/** Tags as typed in one field, split on commas. */
export const parseTags = (text: string) =>
  text
    .split(",")
    .map((tag) => tag.trim())
    .filter(Boolean);
