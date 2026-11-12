// The desktop app's own updates, from GitHub releases through electron-updater. It looks 15 seconds after the app
// starts and then every hour, downloads by itself where the app can install by itself, and installs only when
// asked: the server asks once nothing is busy with Resolve (server/appUpdate.ts). Quitting installs too.
// Electron isn't imported here, so tests drive it with a fake updater.
import type { DesktopUpdateState } from "../shared/protocol.ts";
import {
  initialUpdateState,
  notAppliedVersion,
  onAvailable,
  onCheckFailed,
  onCheckStart,
  onDownloaded,
  onDownloadFailed,
  onInstallFailed,
  onNotAvailable,
  onProgress,
  releaseNoteLines,
  type InstallRecord,
} from "./update-state.ts";

/** The part of electron-updater's autoUpdater this uses. */
export interface Updater {
  autoDownload: boolean;
  autoInstallOnAppQuit: boolean;
  disableDifferentialDownload: boolean;
  checkForUpdates(): Promise<unknown>;
  quitAndInstall(isSilent?: boolean, isForceRunAfter?: boolean): void;
  // Each event passes its own kind of argument.
  on(event: string, listener: (...args: any[]) => void): unknown;
}

type UpdateInfo = { version: string; releaseNotes?: string | { version: string; note: string | null }[] | null };

export interface UpdatesOptions {
  updater: Updater;
  currentVersion: string;
  /** Whether the app can replace itself. Mac apps without a Developer ID can't: the user downloads by hand. */
  canInstall: boolean;
  /** Where to download a release by hand. */
  releasesUrl: string;
  /** The install record kept between runs (desktop/update-state.ts); null deletes it. */
  readRecord(): unknown;
  writeRecord(record: InstallRecord | null): void;
  /** Stops the server and every process it started, so the installer can replace their files. */
  stopServer(): Promise<void>;
  /** Starts the server again after an install that didn't start. */
  resumeServer(): Promise<void>;
  onState(state: DesktopUpdateState): void;
  log?(line: string): void;
  now?(): string;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

export const FIRST_CHECK_MS = 15_000;
export const CHECK_EVERY_MS = 60 * 60 * 1000;

export function createUpdates(options: UpdatesOptions) {
  const { updater, currentVersion } = options;
  const now = options.now ?? (() => new Date().toISOString());
  const log = options.log ?? (() => {});

  const notApplied = notAppliedVersion(options.readRecord(), currentVersion);
  if (notApplied) log(`The install of ${notApplied} didn't apply: still on ${currentVersion}.`);
  else options.writeRecord(null);

  let state = initialUpdateState(currentVersion, notApplied);
  let checking = false;
  let installing = false;
  let installError: string | null = null;
  const timers: ReturnType<typeof setTimeout>[] = [];

  function set(next: DesktopUpdateState) {
    if (next.status !== state.status || next.error !== state.error) {
      log(`${next.status}${next.availableVersion ? ` ${next.availableVersion}` : ""}${next.error ? `: ${next.error}` : ""}`);
    }
    state = next;
    options.onState(state);
  }

  updater.autoDownload = options.canInstall;
  // The app quits with its server stopped (desktop/main.ts), so the installer can run then too.
  updater.autoInstallOnAppQuit = options.canInstall;

  updater.on("checking-for-update", () => set(onCheckStart(state)));
  updater.on("update-available", (info: UpdateInfo) =>
    set(onAvailable(state, info.version, releaseNoteLines(info.releaseNotes), now(), options.canInstall ? null : options.releasesUrl)),
  );
  updater.on("update-not-available", () => set(onNotAvailable(state, now())));
  updater.on("download-progress", (progress: { percent: number }) => {
    if (state.status !== "downloading" || Math.floor(progress.percent) !== state.percent) set(onProgress(state, progress.percent));
  });
  updater.on("update-downloaded", (info: UpdateInfo) => set(onDownloaded(state, info.version)));
  updater.on("error", (error: Error) => {
    const message = errorText(error);
    if (installing) installError = message;
    else if (state.status === "downloading" || (state.status === "available" && options.canInstall)) set(onDownloadFailed(state, message));
    else set(onCheckFailed(state, message, now()));
  });

  async function check() {
    if (checking || installing || state.status === "downloading") return;
    checking = true;
    try {
      const result = (await updater.checkForUpdates()) as { downloadPromise?: Promise<unknown> | null } | null;
      // A failed download is reported through the "error" event too.
      void result?.downloadPromise?.catch(() => {});
    } catch (error) {
      // The "error" event usually said it already.
      if (state.status === "checking") set(onCheckFailed(state, errorText(error), now()));
    } finally {
      checking = false;
    }
  }

  return {
    state: () => state,
    start() {
      timers.push(setTimeout(() => void check(), FIRST_CHECK_MS), setInterval(() => void check(), CHECK_EVERY_MS));
    },
    stop() {
      for (const timer of timers) clearTimeout(timer);
    },
    check,
    /**
     * Restarts into the downloaded release: stops the server and what it started, then runs the installer, which
     * opens the app again. The caller makes sure nothing is busy with Resolve. False when there's nothing to install.
     */
    async install(): Promise<boolean> {
      const version = state.downloadedVersion;
      if (installing || state.status !== "downloaded" || !version) return false;
      installing = true;
      installError = null;
      log(`Installing ${version}`);
      options.writeRecord({ from: currentVersion, to: version });
      try {
        await options.stopServer();
        updater.quitAndInstall(true, true);
      } catch (error) {
        installError = errorText(error);
      }
      // electron-updater reports a failed start through "error", before quitAndInstall returns.
      if (installError === null) return true;
      const message = `Couldn't start the installer: ${installError}`;
      installing = false;
      options.writeRecord(null);
      set(onInstallFailed(state, message));
      await options.resumeServer();
      return false;
    },
    /** Before the app quits: electron-updater installs a downloaded release then. */
    quitting() {
      if (!installing && options.canInstall && state.status === "downloaded" && state.downloadedVersion) {
        options.writeRecord({ from: currentVersion, to: state.downloadedVersion });
      }
    },
  };
}

export type Updates = ReturnType<typeof createUpdates>;
