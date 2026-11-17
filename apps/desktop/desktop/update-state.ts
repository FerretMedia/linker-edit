// How the desktop app's update state moves from one status to the next, like T3 Code's updateMachine.ts: plain
// functions with no Electron in them, so they're tested on their own. desktop/updater.ts calls them as
// electron-updater reports what it's doing.
import type { DesktopUpdate, DesktopUpdateState } from "../shared/protocol.ts";

export function initialUpdateState(currentVersion: string, notApplied: string | null = null): DesktopUpdateState {
  return {
    status: "idle",
    currentVersion,
    availableVersion: null,
    downloadedVersion: null,
    percent: null,
    notes: [],
    checkedAt: null,
    error: null,
    downloadUrl: null,
    notApplied,
  };
}

export function onCheckStart(state: DesktopUpdateState): DesktopUpdateState {
  // A downloaded update stays ready to install while it looks for a newer one.
  if (state.downloadedVersion) return { ...state, error: null };
  return { ...state, status: "checking", percent: null, error: null };
}

export function onCheckFailed(state: DesktopUpdateState, message: string, now: string): DesktopUpdateState {
  if (state.downloadedVersion) return { ...state, status: "downloaded", percent: 100, checkedAt: now };
  return { ...state, status: "error", percent: null, error: message, checkedAt: now };
}

/** `downloadUrl` is set when the app can't install it by itself and the user downloads it by hand. */
export function onAvailable(
  state: DesktopUpdateState,
  version: string,
  notes: string[],
  now: string,
  downloadUrl: string | null = null,
): DesktopUpdateState {
  if (state.downloadedVersion === version) {
    return { ...state, status: "downloaded", availableVersion: version, percent: 100, checkedAt: now, error: null };
  }
  return {
    ...state,
    status: "available",
    availableVersion: version,
    downloadedVersion: null,
    percent: null,
    notes,
    checkedAt: now,
    error: null,
    downloadUrl,
  };
}

export function onNotAvailable(state: DesktopUpdateState, now: string): DesktopUpdateState {
  if (state.downloadedVersion) return { ...state, status: "downloaded", percent: 100, checkedAt: now, error: null };
  return { ...state, status: "up-to-date", availableVersion: null, percent: null, notes: [], checkedAt: now, error: null, downloadUrl: null };
}

export function onProgress(state: DesktopUpdateState, percent: number): DesktopUpdateState {
  return { ...state, status: "downloading", percent: Math.max(0, Math.min(100, Math.floor(percent))), error: null };
}

export function onDownloaded(state: DesktopUpdateState, version: string): DesktopUpdateState {
  return { ...state, status: "downloaded", availableVersion: version, downloadedVersion: version, percent: 100, error: null };
}

export function onDownloadFailed(state: DesktopUpdateState, message: string): DesktopUpdateState {
  // With a release found, "available" lets the next check download it again.
  return { ...state, status: state.availableVersion ? "available" : "error", percent: null, error: message };
}

/** The installer didn't start; the update is still downloaded, so the user can try again. */
export function onInstallFailed(state: DesktopUpdateState, message: string): DesktopUpdateState {
  return { ...state, status: "downloaded", error: message };
}

/** For pages of older Linker-Edits, which only knew a ready update or one to download by hand. */
export function legacyUpdate(state: DesktopUpdateState): DesktopUpdate | null {
  if (state.status === "downloaded" && state.downloadedVersion) return { version: state.downloadedVersion, state: "ready" };
  if (state.downloadUrl && state.availableVersion) return { version: state.availableVersion, state: "download", url: state.downloadUrl };
  return null;
}

/** Written just before installing, read when the app starts again: see notAppliedVersion. */
export interface InstallRecord {
  from: string;
  to: string;
}

/**
 * The version an install was meant to bring, when the app started again still on the version it was installed
 * from: the installer failed or was stopped. Null when the install worked, or the record is unreadable.
 */
export function notAppliedVersion(record: unknown, current: string): string | null {
  if (!record || typeof record !== "object") return null;
  const { from, to } = record as Partial<InstallRecord>;
  return typeof from === "string" && typeof to === "string" && from === current && to !== current ? to : null;
}

const MAX_NOTES = 6;
const MAX_NOTE_LENGTH = 140;

const ENTITIES: Record<string, string> = { amp: "&", lt: "<", gt: ">", quot: '"', apos: "'", nbsp: " " };

function plainText(html: string): string {
  return html
    .replace(/<[^>]*>/g, "")
    .replace(/&(#x[0-9a-f]+|#\d+|[a-z]+);/gi, (entity, name: string) => {
      if (name[0] === "#") {
        const code = name[1] === "x" || name[1] === "X" ? parseInt(name.slice(2), 16) : Number(name.slice(1));
        return Number.isFinite(code) ? String.fromCodePoint(code) : entity;
      }
      return ENTITIES[name.toLowerCase()] ?? entity;
    })
    .replace(/\s+/g, " ")
    .trim();
}

function shorten(line: string): string {
  return line.length > MAX_NOTE_LENGTH ? `${line.slice(0, MAX_NOTE_LENGTH - 1).trimEnd()}…` : line;
}

/**
 * A few short lines of what's new, from the release notes electron-updater found: the GitHub release's body as
 * HTML. Releases have GitHub's generated notes, a list of the merged pull requests, "Title by @user in #12".
 */
export function releaseNoteLines(notes: string | { version: string; note: string | null }[] | null | undefined): string[] {
  if (!notes) return [];
  const html = typeof notes === "string" ? notes : notes.map((entry) => entry.note ?? "").join("\n");
  const items = [...html.matchAll(/<li[^>]*>([\s\S]*?)<\/li>/gi)].map((match) => plainText(match[1]!));
  const lines = items.length
    ? items.map((item) => item.replace(/ by @[\w-]+(?:\[bot\])? in \S+$/, "").replace(/ in #\d+$/, ""))
    : [...html.matchAll(/<p[^>]*>([\s\S]*?)<\/p>/gi)].map((match) => plainText(match[1]!));
  const kept = lines.filter((line) => line && !/^full changelog\b/i.test(line) && !/^new contributors\b/i.test(line));
  // Without any HTML, the notes are plain text, one line each.
  const plain = kept.length || /<[a-z]/i.test(html) ? kept : html.split("\n").map(plainText).filter(Boolean);
  return plain.slice(0, MAX_NOTES).map(shorten);
}
