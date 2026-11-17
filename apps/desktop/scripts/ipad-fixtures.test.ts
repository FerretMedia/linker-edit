import { once } from "node:events";
import * as fs from "node:fs";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { WebSocket } from "ws";
import { applyServerMessage } from "../src/lib/appState.ts";
import type { AppState, ClientMessage, ResolveStatus } from "../shared/protocol.ts";
import { createApp } from "../server/app.ts";
import { createServer } from "../server/http.ts";
import type { PreviewEvent, PreviewRenderer } from "../server/preview/preview.ts";
import { NEW_THREAD_SIDEBAR } from "../server/projects/store.ts";
import { memoryProjectStore, workspaceProject } from "../server/projects/testing.ts";
import type { TurnSink } from "../server/providers/types.ts";

/**
 * Records what the server really sends over /ws, for the iPad app's tests (apps/ipad): its Swift models must
 * decode every file, so a change to shared/protocol.ts that breaks the iPad shows up there. Run it again after
 * changing the protocol, then push: the iPad CI decodes the new files.
 *
 *   RECORD_IPAD_FIXTURES=1 npm test -- scripts/ipad-fixtures.test.ts
 */
const RECORD = process.env.RECORD_IPAD_FIXTURES === "1";
const FIXTURES = path.resolve(import.meta.dirname, "../../ipad/Packages/LinkerEditKit/Tests/Fixtures");

const OPEN: ResolveStatus = {
  state: "connected",
  product: "DaVinci Resolve Studio",
  version: "21.1.2",
  project: "Interview",
  projectId: "resolve-interview",
  database: { type: "Disk", name: "Local Database" },
  timeline: "Cut",
};

/** What the iPad sends, typed against the protocol here and compared with what the Swift models encode. */
const CLIENT_MESSAGES = {
  send: { type: "send", text: "Cut the silences" },
  stop: { type: "stop" },
  approve: { type: "approve", id: "request-1", allow: true },
  deny: { type: "approve", id: "request-1", allow: false },
  select_thread: { type: "select_thread", threadId: "thread-1" },
  new_thread: { type: "new_thread", projectId: "project-1" },
  set_approval_mode: { type: "set_approval_mode", mode: "ask" },
  set_provider: { type: "set_provider", provider: "codex" },
  refresh_resolve: { type: "refresh_resolve" },
  render_preview: { type: "render_preview" },
  render_preview_timeline: { type: "render_preview", timelineId: "timeline-1" },
  stop_preview: { type: "stop_preview" },
  mark_read: { type: "mark_read", threadId: "thread-1", read: true },
} satisfies Record<string, ClientMessage>;

/** Ids and times change on every run, and the computer's name is the developer's: fixed values keep diffs small. */
function stable(raw: string, ids: Map<string, string>): string {
  return raw
    .replaceAll(os.hostname(), "edit-pc")
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/g, (id) => {
      if (!ids.has(id)) ids.set(id, `00000000-0000-4000-8000-${String(ids.size + 1).padStart(12, "0")}`);
      return ids.get(id)!;
    })
    .replace(/\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d\.\d{3}Z/g, "2026-11-17T09:00:00.000Z");
}

describe.runIf(RECORD)("iPad fixtures", () => {
  it("records a session over /ws", async () => {
    let allow: Promise<boolean> | null = null;
    const turn = async (sink: TurnSink) => {
      sink.upsert({ id: "a1", kind: "assistant", text: "Looking at" });
      sink.upsert({ id: "a1", kind: "assistant", text: "Looking at the timeline first." });
      sink.upsert({ id: "t1", kind: "tool", name: "mcp__linker__list_timelines", input: {}, status: "running" });
      sink.upsert({ id: "t1", kind: "tool", name: "mcp__linker__list_timelines", input: {}, status: "done", output: '["Cut"]' });
      allow = sink.requestApproval("mcp__linker__create_timeline", { name: "AI - Cut", from: "Cut" });
      const allowed = await allow;
      sink.upsert({
        id: "t2",
        kind: "tool",
        name: "mcp__linker__create_timeline",
        input: { name: "AI - Cut", from: "Cut" },
        status: allowed ? "done" : "error",
        output: "Created AI - Cut with 41 clips.",
      });
      sink.notice("Backed up the project before the first change.");
      sink.upsert({ id: "a2", kind: "assistant", text: "Done: **AI - Cut** has the 12 long pauses taken out." });
    };
    const renders: ((event: PreviewEvent) => void)[] = [];
    const preview: PreviewRenderer = {
      render: (_timelineId, onEvent) => (renders.push(onEvent), { stop() {} }),
      file: () => null,
    };
    const thread = { ...NEW_THREAD_SIDEBAR, provider: "claude" as const, resumeId: null, context: null, archivedAt: null };
    const app = createApp({
      version: "0.0.0",
      store: memoryProjectStore([
        {
          project: { ...workspaceProject, id: "project-1", name: "Interview", link: { kind: "local", name: "Interview", projectId: "resolve-interview" } },
          threads: [{ ...thread, id: "thread-1", title: "Silences", items: [], createdAt: "2026-11-17T09:00:00.000Z", updatedAt: "2026-11-17T09:00:00.000Z" }],
        },
        {
          project: { ...workspaceProject, id: "project-2", dir: "/promo", name: "Promo", link: { kind: "local", name: "Promo", projectId: "resolve-promo" } },
          threads: [{ ...thread, id: "thread-2", title: "Titles", items: [], createdAt: "2026-11-16T09:00:00.000Z", updatedAt: "2026-11-16T09:00:00.000Z" }],
        },
      ]),
      createSession: () => ({ send: (_text, sink) => turn(sink), resumeId: () => null }),
      checkResolve: async () => OPEN,
      preview,
    });
    const dist = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-ipad-fixtures-"));
    const server = createServer(app, dist, "secret");
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const port = (server.address() as AddressInfo).port;

    const raw: string[] = [];
    let state: AppState | null = null;
    const waiters: { check: (state: AppState, types: string[]) => boolean; resolve: () => void }[] = [];
    const types = () => raw.map((message) => (JSON.parse(message) as { type: string }).type);
    const socket = new WebSocket(`ws://127.0.0.1:${port}/ws`, { origin: `http://127.0.0.1:${port}` });
    socket.on("message", (data) => {
      raw.push(String(data));
      state = applyServerMessage(state, JSON.parse(String(data)));
      for (const waiter of [...waiters]) {
        if (state && waiter.check(state, types())) {
          waiters.splice(waiters.indexOf(waiter), 1);
          waiter.resolve();
        }
      }
    });
    const until = (check: (state: AppState, types: string[]) => boolean) =>
      new Promise<void>((resolve) => (state && check(state, types()) ? resolve() : waiters.push({ check, resolve })));
    const send = (message: ClientMessage) => socket.send(JSON.stringify(message));

    await until(() => true);
    send({ type: "refresh_resolve" });
    await until((s) => s.resolve.state === "connected");
    send({ type: "select_thread", threadId: "thread-1" });
    await until((s) => s.threadId === "thread-1");
    send({ type: "set_approval_mode", mode: "ask" });
    await until((s) => s.approvalMode === "ask");
    send({ type: "send", text: "Cut the silences" });
    await until((s) => s.approvals.length > 0);
    send({ type: "approve", id: state!.approvals[0]!.id, allow: true });
    await until((s) => !s.busy && s.items.some((item) => item.id === "a2"));
    send({ type: "render_preview" });
    await until((s) => s.preview.state === "rendering");
    renders[0]!({ event: "start", timeline: { id: "timeline-1", name: "AI - Cut" } });
    renders[0]!({ event: "progress", percent: 40 });
    renders[0]!({ event: "done", file: "/previews/preview-1.mp4" });
    await until((s) => s.preview.state === "ready");
    // Promo isn't the project open in Resolve, so a message there goes back to the composer.
    send({ type: "select_thread", threadId: "thread-2" });
    await until((s) => s.threadId === "thread-2");
    send({ type: "send", text: "Add titles" });
    await until((_s, sent) => sent.includes("refused") && sent.includes("toast"));
    socket.close();
    server.closeAllConnections();
    server.close();
    expect(allow).not.toBeNull();

    const ids = new Map<string, string>();
    fs.rmSync(FIXTURES, { recursive: true, force: true });
    fs.mkdirSync(path.join(FIXTURES, "server"), { recursive: true });
    fs.mkdirSync(path.join(FIXTURES, "client"), { recursive: true });
    raw.forEach((message, index) => {
      const type = (JSON.parse(message) as { type: string }).type;
      const json = JSON.stringify(JSON.parse(stable(message, ids)), null, 2);
      fs.writeFileSync(path.join(FIXTURES, "server", `${String(index + 1).padStart(3, "0")}-${type}.json`), `${json}\n`);
    });
    for (const [name, message] of Object.entries(CLIENT_MESSAGES)) {
      fs.writeFileSync(path.join(FIXTURES, "client", `${name}.json`), `${JSON.stringify(message, null, 2)}\n`);
    }
    expect(types()).toEqual(
      expect.arrayContaining(["snapshot", "meta", "item", "refused", "toast"]),
    );
  });
});
