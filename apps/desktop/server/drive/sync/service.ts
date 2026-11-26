import type { DriveSync } from "../../../shared/protocol.ts";
import type { DriveLink } from "../../projects/store.ts";
import type { DriveClient } from "../client.ts";
import { syncProject, SyncStopped } from "./sync.ts";

/** How often a mirrored project is synced again: Drive's changes feed and a scan of the folder are both cheap. */
const EVERY = 30_000;
/** After a pass that left a file that was changing under it. */
const SOON = 5_000;

export interface DriveSyncOptions {
  drive: DriveClient;
  /** Drive's id for the Google account signed in here; null when signed out. */
  accountId: () => string | null;
  computer: string;
  every?: number;
  log?: (line: string) => void;
}

interface Mirrored {
  dir: string;
  link: DriveLink;
  status: DriveSync;
  timer: NodeJS.Timeout | null;
  running: boolean;
  /** Asked for again while a pass ran. */
  again: boolean;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const percent = (done: number, total: number) => (total > 0 ? Math.floor((done * 100) / total) : 0);

export type DriveSyncService = ReturnType<typeof createDriveSync>;

/** Syncs each project this computer mirrors, one pass at a time per project, again and again. */
export function createDriveSync(options: DriveSyncOptions) {
  const every = options.every ?? EVERY;
  const log = options.log ?? ((line: string) => console.log(`Drive sync: ${line}`));
  const projects = new Map<string, Mirrored>();
  const statusListeners = new Set<() => void>();
  const changeListeners = new Set<(projectId: string, files: string[]) => void>();

  function set(entry: Mirrored, status: DriveSync) {
    if (JSON.stringify(entry.status) === JSON.stringify(status)) return;
    entry.status = status;
    for (const listener of statusListeners) listener();
  }

  function schedule(id: string, entry: Mirrored, ms: number) {
    if (entry.timer) clearTimeout(entry.timer);
    entry.timer = setTimeout(() => void pass(id), ms);
    entry.timer.unref();
  }

  async function pass(id: string) {
    const entry = projects.get(id);
    if (!entry) return;
    if (entry.running) {
      entry.again = true;
      return;
    }
    const account = options.accountId();
    if (account !== entry.link.accountId) {
      set(entry, {
        state: "error",
        message: account ? "This project is on another Google account's Drive. Sign in to that one to sync it." : "Sign in to Google Drive in Settings to sync this project.",
      });
      return schedule(id, entry, every);
    }
    entry.running = true;
    let next = every;
    try {
      const result = await syncProject({
        drive: options.drive,
        dir: entry.dir,
        folderId: entry.link.folderId,
        computer: options.computer,
        // Only a new whole percent goes out: the sidebar shows no more.
        onProgress: (done, total) => {
          const now = entry.status;
          if (now.state !== "syncing" || now.total !== total || percent(now.done, total) !== percent(done, total)) set(entry, { state: "syncing", done, total });
        },
        log: (line) => log(`${entry.dir}: ${line}`),
      });
      if (result.changed.length > 0) for (const listener of changeListeners) listener(id, result.changed);
      const [first, ...more] = result.failures;
      if (first) {
        set(entry, { state: "error", message: `Couldn't sync ${more.length > 0 ? `${more.length + 1} files, such as ` : ""}${first.path}. ${first.message}` });
      } else if (result.skipped.length > 0) {
        next = SOON;
      } else {
        set(entry, { state: "synced" });
      }
    } catch (error) {
      // fetch gives up with a TypeError when Drive can't be reached.
      const message = error instanceof SyncStopped ? error.message : error instanceof TypeError ? "Can't reach Google Drive. Trying again shortly." : errorText(error);
      // Once, not every pass while it lasts.
      if (entry.status.state !== "error" || entry.status.message !== message) log(`${entry.dir}: ${errorText(error)}`);
      set(entry, { state: "error", message });
    } finally {
      entry.running = false;
    }
    // Stopped, or started again with another folder, while it ran.
    if (projects.get(id) !== entry) return;
    schedule(id, entry, entry.again ? 0 : next);
    entry.again = false;
  }

  return {
    /** Starts syncing a project this computer mirrors, or carries on when it already is. */
    start(id: string, dir: string, link: DriveLink) {
      const known = projects.get(id);
      if (known && known.dir === dir && JSON.stringify(known.link) === JSON.stringify(link)) return;
      if (known?.timer) clearTimeout(known.timer);
      projects.set(id, { dir, link, status: { state: "syncing", done: 0, total: 0 }, timer: null, running: false, again: false });
      for (const listener of statusListeners) listener();
      void pass(id);
    },

    stop(id: string) {
      const entry = projects.get(id);
      if (!entry) return;
      if (entry.timer) clearTimeout(entry.timer);
      projects.delete(id);
      for (const listener of statusListeners) listener();
    },

    status(id: string): DriveSync | undefined {
      return projects.get(id)?.status;
    },

    /** Syncs every project now, as after signing in to Google. */
    wake() {
      for (const id of projects.keys()) void pass(id);
    },

    onStatus(listener: () => void) {
      statusListeners.add(listener);
    },

    /** Files a pass wrote, moved or deleted in a project's folder, such as another computer's linker-edit.json. */
    onChanged(listener: (projectId: string, files: string[]) => void) {
      changeListeners.add(listener);
    },
  };
}
