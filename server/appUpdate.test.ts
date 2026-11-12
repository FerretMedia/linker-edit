import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DesktopUpdateState, ServerMessage, ServerToDesktop } from "../shared/protocol.ts";
import { createApp } from "./app.ts";
import { createAppUpdates } from "./appUpdate.ts";
import { memoryProjectStore, workspaceProject } from "./projects/testing.ts";

const state = (status: DesktopUpdateState["status"]): DesktopUpdateState => ({
  status,
  currentVersion: "2026.1111.2",
  availableVersion: status === "downloaded" ? "2026.1112.0" : null,
  downloadedVersion: status === "downloaded" ? "2026.1112.0" : null,
  percent: null,
  notes: [],
  checkedAt: null,
  error: null,
  downloadUrl: null,
  notApplied: null,
});

function setup() {
  const sent: ServerToDesktop[] = [];
  let busy = false;
  const changes = vi.fn();
  const updates = createAppUpdates({ send: (message) => sent.push(message), busy: () => busy, pollMs: 1000 });
  updates.onStatus(changes);
  return { updates, sent, changes, setBusy: (value: boolean) => (busy = value) };
}

describe("createAppUpdates", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("has nothing to show until the desktop app says where it is", () => {
    const { updates, changes } = setup();
    expect(updates.status()).toBeNull();
    updates.receive({ type: "update_state", state: state("up-to-date") });
    expect(updates.status()).toMatchObject({ status: "up-to-date", waiting: false });
    expect(changes).toHaveBeenCalledTimes(1);
  });

  it("asks the app to check", () => {
    const { updates, sent } = setup();
    updates.check();
    expect(sent).toEqual([{ type: "check_update" }]);
  });

  it("installs right away when Resolve is free", () => {
    const { updates, sent } = setup();
    updates.receive({ type: "update_state", state: state("downloaded") });
    updates.install();
    expect(sent).toEqual([{ type: "install_update" }]);
  });

  it("does nothing without a downloaded update", () => {
    const { updates, sent } = setup();
    updates.install();
    updates.receive({ type: "update_state", state: state("downloading") });
    updates.install();
    expect(sent).toEqual([]);
  });

  it("waits for a running turn to end", async () => {
    const { updates, sent, setBusy } = setup();
    updates.receive({ type: "update_state", state: state("downloaded") });
    setBusy(true);
    updates.install();
    expect(updates.status()?.waiting).toBe(true);
    await vi.advanceTimersByTimeAsync(3000);
    expect(sent).toEqual([]);
    setBusy(false);
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent).toEqual([{ type: "install_update" }]);
    expect(updates.status()?.waiting).toBe(false);
  });

  it("can be told not to restart after all", async () => {
    const { updates, sent, setBusy } = setup();
    updates.receive({ type: "update_state", state: state("downloaded") });
    setBusy(true);
    updates.install();
    updates.cancel();
    setBusy(false);
    await vi.advanceTimersByTimeAsync(5000);
    expect(sent).toEqual([]);
    expect(updates.status()?.waiting).toBe(false);
  });

  it("waits the same way when a page of an older Linker-Edit asks through the app", async () => {
    const { updates, sent, setBusy } = setup();
    updates.receive({ type: "update_state", state: state("downloaded") });
    setBusy(true);
    updates.receive({ type: "install_requested" });
    expect(updates.status()?.waiting).toBe(true);
    setBusy(false);
    await vi.advanceTimersByTimeAsync(1000);
    expect(sent).toEqual([{ type: "install_update" }]);
  });

  it("stops waiting when a newer release replaces the downloaded one", async () => {
    const { updates, sent, setBusy } = setup();
    updates.receive({ type: "update_state", state: state("downloaded") });
    setBusy(true);
    updates.install();
    updates.receive({ type: "update_state", state: state("downloading") });
    setBusy(false);
    await vi.advanceTimersByTimeAsync(5000);
    expect(sent).toEqual([]);
  });
});

describe("the app and its desktop updates", () => {
  it("shows the update to pages and passes their asks on", async () => {
    const sent: ServerToDesktop[] = [];
    const appUpdates = createAppUpdates({ send: (message) => sent.push(message), busy: () => app.busy() });
    const app = createApp({
      version: "2026.1111.2",
      store: memoryProjectStore([{ project: workspaceProject }]),
      // No turns here.
      createSession: () => ({ send: async () => {}, resumeId: () => null }),
      checkResolve: async () => ({ state: "not_running" }),
      appUpdates,
    });
    const messages: ServerMessage[] = [];
    const client = { send: (data: string) => messages.push(JSON.parse(data) as ServerMessage) };
    app.connect(client);
    const appUpdate = () => {
      const last = messages.findLast((message) => message.type === "meta" || message.type === "snapshot")!;
      return last.type === "meta" ? last.meta.appUpdate : last.type === "snapshot" ? last.state.appUpdate : undefined;
    };
    expect(appUpdate()).toBeNull();

    appUpdates.receive({ type: "update_state", state: state("downloaded") });
    expect(appUpdate()).toMatchObject({ status: "downloaded", downloadedVersion: "2026.1112.0", waiting: false });

    app.handleMessage(client, { type: "check_app_update" });
    app.handleMessage(client, { type: "install_app_update" });
    expect(sent).toEqual([{ type: "check_update" }, { type: "install_update" }]);
  });
});
