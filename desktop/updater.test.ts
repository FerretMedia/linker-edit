import { EventEmitter } from "node:events";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DesktopUpdateState } from "../shared/protocol.ts";
import type { InstallRecord } from "./update-state.ts";
import { CHECK_EVERY_MS, createUpdates, FIRST_CHECK_MS, type UpdatesOptions } from "./updater.ts";

/** Stands in for electron-updater's autoUpdater; tests emit its events. */
class FakeUpdater extends EventEmitter {
  autoDownload = true;
  autoInstallOnAppQuit = true;
  disableDifferentialDownload = false;
  checks = 0;
  installs: [boolean | undefined, boolean | undefined][] = [];
  failInstall: string | null = null;
  onCheck: () => void = () => {};
  async checkForUpdates() {
    this.checks++;
    this.emit("checking-for-update");
    this.onCheck();
    return null;
  }
  quitAndInstall(isSilent?: boolean, isForceRunAfter?: boolean) {
    this.installs.push([isSilent, isForceRunAfter]);
    if (this.failInstall) this.emit("error", new Error(this.failInstall));
  }
}

function setup(overrides: Partial<UpdatesOptions> = {}, record: unknown = null) {
  const updater = new FakeUpdater();
  const states: DesktopUpdateState[] = [];
  const records: (InstallRecord | null)[] = [];
  const calls: string[] = [];
  const updates = createUpdates({
    updater,
    currentVersion: "2026.1111.2",
    canInstall: true,
    releasesUrl: "https://example.com/releases",
    readRecord: () => record,
    writeRecord: (value) => records.push(value),
    stopServer: async () => void calls.push("stop"),
    resumeServer: async () => void calls.push("resume"),
    onState: (state) => states.push(state),
    now: () => "2026-11-12T10:00:00.000Z",
    ...overrides,
  });
  return { updater, updates, states, records, calls };
}

const downloaded = (updater: FakeUpdater) => {
  updater.emit("update-available", { version: "2026.1112.0", releaseNotes: "<ul><li>Faster previews by @a in #9</li></ul>" });
  updater.emit("update-downloaded", { version: "2026.1112.0" });
};

describe("createUpdates", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("downloads by itself where it can install, and installs on quit", () => {
    const { updater } = setup();
    expect(updater.autoDownload).toBe(true);
    expect(updater.autoInstallOnAppQuit).toBe(true);
  });

  it("only links to the release where it can't install by itself", () => {
    const { updater, updates } = setup({ canInstall: false });
    expect(updater.autoDownload).toBe(false);
    expect(updater.autoInstallOnAppQuit).toBe(false);
    updater.emit("update-available", { version: "2026.1112.0" });
    expect(updates.state()).toMatchObject({ status: "available", downloadUrl: "https://example.com/releases" });
  });

  it("checks 15 seconds after starting, then every hour", async () => {
    const { updater, updates } = setup();
    updates.start();
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_MS);
    expect(updater.checks).toBe(1);
    await vi.advanceTimersByTimeAsync(CHECK_EVERY_MS);
    expect(updater.checks).toBe(2);
    updates.stop();
  });

  it("follows electron-updater to a downloaded update, with its notes", async () => {
    const { updater, updates, states } = setup();
    updater.onCheck = () => updater.emit("update-available", { version: "2026.1112.0", releaseNotes: "<ul><li>Faster previews by @a in #9</li></ul>" });
    await updates.check();
    updater.emit("download-progress", { percent: 12.5 });
    updater.emit("download-progress", { percent: 12.9 });
    updater.emit("update-downloaded", { version: "2026.1112.0" });
    expect(states.map((state) => state.status)).toEqual(["checking", "available", "downloading", "downloaded"]);
    expect(updates.state()).toMatchObject({ downloadedVersion: "2026.1112.0", notes: ["Faster previews"] });
  });

  it("tells a failed download from a failed check", async () => {
    const { updater, updates } = setup();
    updater.onCheck = () => updater.emit("error", new Error("offline"));
    await updates.check();
    expect(updates.state()).toMatchObject({ status: "error", error: "offline" });
    updater.onCheck = () => updater.emit("update-available", { version: "2026.1112.0" });
    await updates.check();
    updater.emit("error", new Error("connection reset"));
    expect(updates.state()).toMatchObject({ status: "available", error: "connection reset" });
  });

  it("doesn't check while downloading", async () => {
    const { updater, updates } = setup();
    updater.emit("update-available", { version: "2026.1112.0" });
    updater.emit("download-progress", { percent: 5 });
    await updates.check();
    expect(updater.checks).toBe(0);
  });

  it("stops the server, keeps a record, then installs silently and opens the app again", async () => {
    const { updater, updates, records, calls } = setup();
    downloaded(updater);
    expect(await updates.install()).toBe(true);
    expect(calls).toEqual(["stop"]);
    expect(records.at(-1)).toEqual({ from: "2026.1111.2", to: "2026.1112.0" });
    expect(updater.installs).toEqual([[true, true]]);
    // Quitting for the install doesn't write the record again.
    updates.quitting();
    expect(records).toHaveLength(2);
  });

  it("has nothing to install before a download", async () => {
    const { updater, updates, calls } = setup();
    expect(await updates.install()).toBe(false);
    expect(calls).toEqual([]);
    expect(updater.installs).toEqual([]);
  });

  it("starts the server again when the installer doesn't start", async () => {
    const { updater, updates, records, calls } = setup();
    downloaded(updater);
    updater.failInstall = "spawn EACCES";
    expect(await updates.install()).toBe(false);
    expect(calls).toEqual(["stop", "resume"]);
    expect(records.at(-1)).toBeNull();
    expect(updates.state()).toMatchObject({ status: "downloaded", error: "Couldn't start the installer: spawn EACCES" });
  });

  it("keeps a record when quitting installs the update", () => {
    const { updater, updates, records } = setup();
    updates.quitting();
    expect(records).toEqual([null]);
    downloaded(updater);
    updates.quitting();
    expect(records.at(-1)).toEqual({ from: "2026.1111.2", to: "2026.1112.0" });
  });

  it("says when the last install didn't apply, and forgets a record that did", () => {
    const failed = setup({}, { from: "2026.1111.2", to: "2026.1112.0" });
    expect(failed.updates.state().notApplied).toBe("2026.1112.0");
    expect(failed.records).toEqual([]);
    const worked = setup({}, { from: "2026.1111.1", to: "2026.1111.2" });
    expect(worked.updates.state().notApplied).toBeNull();
    expect(worked.records).toEqual([null]);
  });
});
