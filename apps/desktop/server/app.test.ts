import { describe, expect, it, vi } from "vitest";
import { applyServerMessage } from "../src/lib/appState.ts";
import type { AppState, ClientMessage, HistoryEntry, PlayableTimeline, Provider, ResolveStatus, ServerMessage } from "../shared/protocol.ts";
import { createApp, type App, type AppOptions } from "./app.ts";
import type { AgentTurn, History } from "./history/history.ts";
import { QUIET_MS } from "./resolve/watch.ts";
import type { ResolveCheck } from "./resolveStatus.ts";
import { NEW_THREAD_SIDEBAR, type SavedProject, type SavedThread } from "./projects/store.ts";
import { memoryProjectStore, workspaceProject } from "./projects/testing.ts";
import type { PreviewEvent, PreviewRenderer } from "./preview/preview.ts";
import type { ProviderSession, TurnSink } from "./providers/types.ts";
import type { Settings } from "./settings.ts";
import { createTranscriber } from "./transcribe/transcriber.ts";

type Script = (sink: TurnSink, signal: AbortSignal) => Promise<void>;

/** A client that keeps state the way the UI does, and resolves waits as messages arrive. */
function testClient() {
  let state: AppState | null = null;
  const histories: HistoryEntry[][] = [];
  const refused: string[] = [];
  const toasts: string[] = [];
  const waiters: { check: () => boolean; resolve: () => void }[] = [];
  return {
    histories,
    refused,
    toasts,
    // Like the web UI: every test here also checks that the patches add up to the same state.
    metaPatches: true,
    send(data: string) {
      const message = JSON.parse(data) as ServerMessage;
      if (message.type === "history") histories.push(message.entries);
      if (message.type === "refused") refused.push(message.text);
      if (message.type === "toast") toasts.push(message.text);
      state = applyServerMessage(state, message);
      for (const waiter of [...waiters]) {
        if (waiter.check()) {
          waiters.splice(waiters.indexOf(waiter), 1);
          waiter.resolve();
        }
      }
    },
    get state(): AppState {
      if (!state) throw new Error("No snapshot yet");
      return state;
    },
    until(predicate: (state: AppState) => boolean): Promise<void> {
      const check = () => state !== null && predicate(state);
      if (check()) return Promise.resolve();
      return new Promise((resolve) => waiters.push({ check, resolve }));
    },
  };
}

/** Each send runs the next script, across sessions, so tests read top to bottom. */
function setup(...scripts: Script[]) {
  return setupWith({}, ...scripts);
}

function connected(project: string | null, projectId: string | null = null): ResolveStatus {
  return {
    state: "connected",
    product: "DaVinci Resolve Studio",
    version: "21",
    project,
    projectId,
    database: { type: "Disk", name: "Local Database" },
    timeline: null,
  };
}

type HarnessOptions = Partial<Pick<AppOptions, "checkResolve" | "preview" | "resolveProjects" | "resolveTools" | "backupProject" | "applyTimeline" | "settings" | "transcriber">> & {
  store?: ReturnType<typeof memoryProjectStore>;
  history?: History;
};

/**
 * Sessions get a provider id on their first send, like the real ones. Claude's compact() sends
 * "/compact" and runs the next script, so compactions read top to bottom with the turns.
 */
function setupWith(options: HarnessOptions, ...scripts: Script[]) {
  const sessions: Provider[] = [];
  const resumeIds: (string | undefined)[] = [];
  const prompts: string[] = [];
  const dirs: string[] = [];
  const store = options.store ?? memoryProjectStore([{ project: workspaceProject }]);
  const app = createApp({
    version: "0.0.0",
    store,
    createSession: (provider, _approvalMode, dir, resumeId): ProviderSession => {
      sessions.push(provider);
      resumeIds.push(resumeId);
      dirs.push(dir);
      let id = resumeId ?? null;
      const name = `${provider}-${sessions.length}`;
      const send: ProviderSession["send"] = (text, sink, signal) => {
        prompts.push(text);
        id ??= name;
        const script = scripts.shift();
        if (!script) throw new Error("No script left for this send");
        return script(sink, signal);
      };
      return {
        send,
        ...(provider === "claude" ? { compact: (sink: TurnSink, signal: AbortSignal) => send("/compact", sink, signal) } : {}),
        resumeId: () => id,
      };
    },
    checkResolve: options.checkResolve ?? (async () => ({ state: "not_running" })),
    createHistory: options.history ? () => options.history! : undefined,
    preview: options.preview,
    resolveProjects: options.resolveProjects,
    resolveTools: options.resolveTools,
    backupProject: options.backupProject,
    applyTimeline: options.applyTimeline,
    settings: options.settings,
    transcriber: options.transcriber,
  });
  const client = testClient();
  const disconnect = app.connect(client);
  const send = (message: ClientMessage) => app.handleMessage(client, message);
  const turnDone = () => client.until((state) => !state.busy);
  /** Every turn is over, including one in a thread the client moved away from or deleted. */
  // Polled: a turn in a thread the client left ends without changing anything this client sees.
  const allDone = () => vi.waitFor(() => expect(app.busy()).toBe(false), { timeout: 5000 });
  return { app, client, disconnect, send, turnDone, allDone, sessions, resumeIds, prompts, dirs, store };
}

/** A history that records what it was asked to save. */
function fakeHistory(overrides: Partial<History> = {}): History & { records: (AgentTurn | undefined)[]; commits: string[] } {
  const records: (AgentTurn | undefined)[] = [];
  const commits: string[] = [];
  return {
    records,
    commits,
    record: async (turn) => {
      records.push(turn);
      return turn ? { commit: "abc1234", changes: [] } : null;
    },
    recentContext: async () => "",
    log: async () => [],
    timeline: async () => null,
    mediaFile: () => null,
    commitProjectFile: async (message) => void commits.push(message),
    ...overrides,
  };
}

function savedThread(fields: Partial<SavedThread> = {}): SavedThread {
  return {
    ...NEW_THREAD_SIDEBAR,
    id: "thread-1",
    title: "cut it",
    provider: "claude",
    resumeId: null,
    items: [],
    context: null,
    createdAt: "2026-11-09T10:00:00.000Z",
    updatedAt: "2026-11-09T10:00:00.000Z",
    archivedAt: null,
    ...fields,
  };
}

function gate() {
  let open!: () => void;
  const opened = new Promise<void>((resolve) => (open = resolve));
  return { open, opened };
}

describe("app turns", () => {
  it("runs a turn and marks tools still running at the end as failed", async () => {
    const { client, send, turnDone } = setup(async (sink) => {
      sink.upsert({ id: "t1", kind: "tool", name: "Read", input: {}, status: "running" });
      sink.upsert({ id: "a1", kind: "assistant", text: "Done" });
    });
    send({ type: "send", text: "  hello  " });
    expect(client.state.busy).toBe(true);
    await turnDone();
    expect(client.state.items).toMatchObject([
      { kind: "user", text: "hello" },
      { id: "t1", kind: "tool", status: "error" },
      { id: "a1", kind: "assistant", text: "Done" },
    ]);
  });

  it("shows a provider error, but not one caused by Stop", async () => {
    const { client, send, turnDone } = setup(
      async () => {
        throw new Error("CLI not logged in");
      },
      (_sink, signal) =>
        new Promise((_resolve, reject) => signal.addEventListener("abort", () => reject(new Error("aborted")))),
    );
    send({ type: "send", text: "one" });
    await turnDone();
    send({ type: "send", text: "two" });
    send({ type: "stop" });
    await turnDone();
    const notices = client.state.items.filter((item) => item.kind === "notice");
    expect(notices).toMatchObject([
      { tone: "error", text: "CLI not logged in" },
      { tone: "info", text: "Stopped." },
    ]);
  });

  it("stops a turn's Resolve tool with it, and keeps Resolve taken until the tool has wound down", async () => {
    let toolSignal: AbortSignal | undefined;
    let finish = () => {};
    let call: App["resolveTool"] = () => Promise.reject(new Error("No app yet"));
    const { app, client, send, turnDone } = setupWith(
      {
        checkResolve: async () => connected(null),
        resolveTools: (_tool, _input, _dir, signal) => {
          toolSignal = signal;
          return new Promise((resolve) => (finish = () => resolve({ text: "stopped" })));
        },
      },
      (_sink, signal) =>
        new Promise<void>((resolve) => {
          void call("render_timeline", {});
          signal.addEventListener("abort", () => resolve());
        }),
    );
    call = app.resolveTool;
    send({ type: "send", text: "render it" });
    await vi.waitFor(() => expect(toolSignal).toBeDefined());
    send({ type: "stop" });
    await turnDone();
    expect(toolSignal!.aborted).toBe(true);
    expect(app.busy()).toBe(true);
    expect(client.state.blocked?.message).toMatch(/stopped turn/);
    expect(await app.resolveTool("list_timelines", {})).toMatchObject({ isError: true });
    finish();
    await client.until(() => !app.busy());
    expect(client.state.blocked).toBeNull();
  });

  it("ignores a send while a turn is running", async () => {
    const { open, opened } = gate();
    const { client, send, turnDone } = setup(() => opened);
    send({ type: "send", text: "first" });
    send({ type: "send", text: "second" });
    open();
    await turnDone();
    expect(client.state.items.map((item) => item.kind === "user" && item.text)).toEqual(["first"]);
  });
});

describe("app approvals", () => {
  async function awaitingApproval() {
    const answers: boolean[] = [];
    const harness = setup(async (sink) => {
      answers.push(await sink.requestApproval("mcp__linker__create_timeline", { name: "Cut" }));
    });
    harness.send({ type: "send", text: "build it" });
    await harness.client.until((state) => state.approvals.length > 0);
    return { ...harness, answers };
  }

  it.each([true, false])("passes the user's answer (%s) to the provider", async (allow) => {
    const { client, send, turnDone, answers } = await awaitingApproval();
    const [request] = client.state.approvals;
    expect(request).toMatchObject({ tool: "mcp__linker__create_timeline", input: { name: "Cut" } });
    send({ type: "approve", id: request!.id, allow });
    await turnDone();
    expect(answers).toEqual([allow]);
    expect(client.state.approvals).toEqual([]);
  });

  it("runs agents' Resolve tools for the thread working in Resolve, asking before changes", async () => {
    const results: { text: string; isError: boolean; images?: string[] }[] = [];
    const ran: string[] = [];
    let call: App["resolveTool"] = () => Promise.reject(new Error("No app yet"));
    const { app, client, send, turnDone } = setupWith(
      {
        checkResolve: async () => connected(null),
        resolveTools: async (tool, input, dir) => {
          ran.push(`${tool} in ${dir}`);
          if (tool === "add_markers") throw new Error("Cut is not an AI timeline.");
          return { text: JSON.stringify({ tool, input }), ...(tool === "list_timelines" ? { images: ["frame.jpg"] } : {}) };
        },
      },
      async () => {
        results.push(await call("list_timelines", {}));
        results.push(await call("set_current_timeline", { timeline: "AI - Cut" }));
        results.push(await call("create_timeline", { name: "Cut" }));
        results.push(await call("add_markers", { timeline: "Cut", markers: [{ frame: 0 }] }));
        results.push(await call("nope", {}));
      },
    );
    call = app.resolveTool;
    expect(await app.resolveTool("list_timelines", {})).toEqual({ text: "No Linker-Edit turn is running, so Resolve calls are refused.", isError: true });
    send({ type: "set_approval_mode", mode: "ask" });
    send({ type: "send", text: "build it" });
    await client.until((state) => state.approvals.length > 0);
    expect(client.state.approvals).toMatchObject([{ tool: "mcp__linker__create_timeline", input: { name: "Cut" } }]);
    send({ type: "approve", id: client.state.approvals[0]!.id, allow: false });
    await client.until((state) => state.approvals.length > 0);
    send({ type: "approve", id: client.state.approvals[0]!.id, allow: true });
    await turnDone();
    expect(results).toEqual([
      { text: JSON.stringify({ tool: "list_timelines", input: {} }), images: ["frame.jpg"], isError: false },
      { text: JSON.stringify({ tool: "set_current_timeline", input: { timeline: "AI - Cut" } }), isError: false },
      { text: "The user declined this action.", isError: true },
      { text: "Cut is not an AI timeline.", isError: true },
      { text: "There is no Resolve tool called nope.", isError: true },
    ]);
    const dir = workspaceProject.dir;
    expect(ran).toEqual([`list_timelines in ${dir}`, `set_current_timeline in ${dir}`, `add_markers in ${dir}`]);
  });

  it("starts with full access: Resolve changes run without asking, and the tools' own checks still refuse", async () => {
    const results: { text: string; isError: boolean }[] = [];
    let call: App["resolveTool"] = () => Promise.reject(new Error("No app yet"));
    const { app, client, send, turnDone } = setupWith(
      {
        checkResolve: async () => connected(null),
        resolveTools: async (tool) => {
          if (tool === "add_markers") throw new Error("Cut is not an AI timeline.");
          return { text: tool };
        },
      },
      async () => {
        results.push(await call("create_timeline", { name: "Cut" }));
        results.push(await call("add_markers", { timeline: "Cut", markers: [{ frame: 0 }] }));
      },
    );
    call = app.resolveTool;
    expect(client.state.approvalMode).toBe("auto");
    send({ type: "send", text: "build it" });
    await turnDone();
    expect(results).toEqual([
      { text: "create_timeline", isError: false },
      { text: "Cut is not an AI timeline.", isError: true },
    ]);
    expect(client.state.approvals).toEqual([]);
  });

  it("keeps supervised mode across restarts", () => {
    const saved: { approvalMode?: "ask" | "auto" } = {};
    const settings = { load: () => ({ ...saved }), save: (changes: typeof saved) => void Object.assign(saved, changes) };
    const first = setupWith({ settings });
    first.send({ type: "set_approval_mode", mode: "ask" });
    expect(saved).toEqual({ approvalMode: "ask" });
    first.disconnect();
    expect(setupWith({ settings }).client.state.approvalMode).toBe("ask");
  });

  it("starts new threads with the agent picked last, also after a restart", () => {
    const saved: Settings = {};
    const settings = { load: () => ({ ...saved }), save: (changes: Settings) => void Object.assign(saved, changes) };
    const first = setupWith({ settings });
    expect(first.client.state.defaultProvider).toBe("claude");
    first.send({ type: "set_default_provider", provider: "codex" });
    expect(saved).toEqual({ defaultProvider: "codex" });
    // The open thread keeps its own agent; only new threads start with Codex.
    expect(first.client.state.provider).toBe("claude");
    first.send({ type: "set_default_provider", provider: "nonsense" as "codex" });
    expect(first.client.state.defaultProvider).toBe("codex");
    first.disconnect();
    expect(setupWith({ settings }).client.state.defaultProvider).toBe("codex");
  });

  it("downloads the speech engine by itself, but not on a computer said to have no Resolve", async () => {
    const saved: Settings = {};
    const settings = { load: () => ({ ...saved }), save: (changes: Settings) => void Object.assign(saved, changes) };
    const prepared: string[] = [];
    let resolve: ResolveStatus = { state: "setup_needed", hint: "Install Resolve Studio." };
    const transcriberWith = () =>
      createTranscriber({
        dir: "/tmp/transcripts",
        ready: () => false,
        prepare: (_onProgress, signal) =>
          new Promise((_resolve, reject) => {
            prepared.push("started");
            signal.addEventListener("abort", () => reject(new Error("aborted")));
          }),
      });

    // A computer that drives an edit rig: the user says so, and the download in progress stops.
    const rig = setupWith({ settings, transcriber: transcriberWith(), checkResolve: async () => resolve });
    rig.app.prepareTranscription();
    expect(prepared).toEqual(["started"]);
    expect(rig.client.state.resolveHere).toBe(true);
    rig.send({ type: "set_resolve_here", resolveHere: false });
    expect(saved).toEqual({ resolveHere: false });
    expect(rig.client.state.resolveHere).toBe(false);
    await rig.client.until((state) => state.transcription.state === "idle");
    rig.disconnect();

    // After a restart nothing downloads, until Resolve turns up on this computer after all.
    const remote = setupWith({ settings, transcriber: transcriberWith(), checkResolve: async () => resolve });
    remote.app.prepareTranscription();
    await remote.app.refreshResolve();
    expect(prepared).toEqual(["started"]);
    expect(remote.client.state.resolveHere).toBe(false);
    resolve = { state: "not_running" };
    await remote.app.refreshResolve();
    expect(saved).toEqual({ resolveHere: true });
    expect(remote.client.state.resolveHere).toBe(true);
    expect(prepared).toEqual(["started", "started"]);
  });

  it("backs the project up once per turn, before its first Resolve change, and changes nothing when it can't", async () => {
    const results: { text: string; isError: boolean }[] = [];
    const backups: string[] = [];
    const ran: string[] = [];
    let fail = false;
    let call: App["resolveTool"] = () => Promise.reject(new Error("No app yet"));
    const { app, client, send, turnDone } = setupWith(
      {
        checkResolve: async () => connected(null),
        resolveTools: async (tool) => {
          ran.push(tool);
          return { text: tool };
        },
        backupProject: async (dir) => {
          backups.push(dir);
          if (fail) throw new Error("Resolve couldn't export the project.");
          return "backups/Doc.drp";
        },
      },
      async () => {
        results.push(await call("list_timelines", {}));
        results.push(await call("create_timeline", { name: "A" }));
        results.push(await call("add_clips", {}));
      },
      async () => {
        results.push(await call("create_timeline", { name: "B" }));
        results.push(await call("create_timeline", { name: "C" }));
      },
    );
    call = app.resolveTool;
    send({ type: "send", text: "one" });
    await turnDone();
    expect(backups).toEqual([workspaceProject.dir]);
    expect(ran).toEqual(["list_timelines", "create_timeline", "add_clips"]);
    expect(client.state.items).toContainEqual(expect.objectContaining({ kind: "notice", text: "Backed up the Resolve project to backups/Doc.drp before changing it." }));

    fail = true;
    send({ type: "send", text: "two" });
    await turnDone();
    // A failed backup is tried again before the next change, and nothing runs without one.
    expect(backups).toHaveLength(3);
    expect(ran).toHaveLength(3);
    expect(results.slice(3)).toEqual([
      {
        text: "Linker-Edit backs the Resolve project up before every change and couldn't this time, so nothing was changed: Resolve couldn't export the project.",
        isError: true,
      },
      expect.objectContaining({ isError: true }),
    ]);
  });

  it("makes an AI timeline the main one when the user asks, after a backup", async () => {
    const order: string[] = [];
    const { client, send } = setupWith({
      backupProject: async () => {
        order.push("backup");
        return "backups/Doc.drp";
      },
      applyTimeline: async (dir, timelineId) => {
        order.push(`apply ${timelineId} in ${dir}`);
        return { main: { id: timelineId, name: "Edit" }, previous: { id: "t1", name: "Edit (before AI 2026-11-12 09.30)" }, was: "AI - Tight" };
      },
    });
    send({ type: "apply_timeline", projectId: workspaceProject.id, timelineId: "t2" });
    await client.until((state) => state.applying === workspaceProject.id);
    await client.until((state) => state.applying === null);
    expect(order).toEqual(["backup", `apply t2 in ${workspaceProject.dir}`]);
    expect(client.toasts).toEqual(['"AI - Tight" is now "Edit". The earlier version is kept as "Edit (before AI 2026-11-12 09.30)".']);
  });

  it("doesn't touch the timelines when the backup fails", async () => {
    const applied: string[] = [];
    const { client, send } = setupWith({
      backupProject: () => Promise.reject(new Error("Resolve couldn't export the project.")),
      applyTimeline: async (_dir, timelineId) => {
        applied.push(timelineId);
        return { main: { id: timelineId, name: "Edit" }, previous: { id: "t1", name: "Edit (old)" }, was: "AI - Tight" };
      },
    });
    send({ type: "apply_timeline", projectId: workspaceProject.id, timelineId: "t2" });
    await client.until((state) => state.applying === workspaceProject.id);
    await client.until((state) => state.applying === null);
    expect(applied).toEqual([]);
    expect(client.toasts).toEqual(["Couldn't make it the main timeline: Resolve couldn't export the project."]);
  });

  it("allows pending approvals when switched to auto", async () => {
    const { send, turnDone, answers } = await awaitingApproval();
    send({ type: "set_approval_mode", mode: "yes" as "auto" });
    expect(answers).toEqual([]);
    send({ type: "set_approval_mode", mode: "auto" });
    await turnDone();
    expect(answers).toEqual([true]);
  });

  it.each<[string, (threadId: string) => ClientMessage]>([
    ["stop", () => ({ type: "stop" })],
    ["set_provider", () => ({ type: "set_provider", provider: "codex" })],
    ["archive_thread", (threadId) => ({ type: "archive_thread", threadId, archived: true })],
    ["delete_thread", (threadId) => ({ type: "delete_thread", threadId })],
  ])("denies pending approvals on %s", async (_name, message) => {
    const { client, send, allDone, answers } = await awaitingApproval();
    send(message(client.state.threadId!));
    expect(client.state.approvals).toEqual([]);
    await allDone();
    expect(answers).toEqual([false]);
  });
});

describe("app chats", () => {
  it("keeps a turn that outlives its deleted thread out of every thread", async () => {
    const answers: boolean[] = [];
    const { client, send, allDone, store } = setup(async (sink) => {
      answers.push(await sink.requestApproval("Write", {}));
      sink.upsert({ id: "late", kind: "assistant", text: "late reply" });
      sink.notice("late notice");
      answers.push(await sink.requestApproval("Write", {}));
    });
    send({ type: "send", text: "go" });
    await client.until((state) => state.approvals.length > 0);
    const deleted = client.state.threadId!;
    send({ type: "delete_thread", threadId: deleted });
    await allDone();
    expect(answers).toEqual([false, false]);
    expect(client.state.threadId).not.toBe(deleted);
    expect(client.state.items).toEqual([]);
    expect(client.state.approvals).toEqual([]);
    expect(store.threads.get("workspace")!.size).toBe(0);
  });

  it("starts a fresh session when the provider changes, and only then", async () => {
    const { client, send, sessions } = setup();
    send({ type: "set_provider", provider: "claude" });
    send({ type: "set_provider", provider: "codex" });
    expect(sessions).toEqual(["claude", "codex"]);
    expect(client.state.provider).toBe("codex");
  });

  it("keeps the chat going when the provider changes, giving the new agent a summary", async () => {
    const { client, send, turnDone, prompts } = setup(
      async (sink) => {
        sink.upsert({ id: "a", kind: "assistant", text: "Built AI - Cut." });
        sink.context({ usedTokens: 40_000, maxTokens: 1_000_000, cacheExpiresAt: null });
      },
      async () => {},
    );
    send({ type: "send", text: "cut it" });
    await turnDone();
    send({ type: "set_provider", provider: "codex" });
    expect(client.state.context).toBeNull();
    expect(client.state.items.at(-1)).toMatchObject({ kind: "notice", text: expect.stringContaining("Switched to Codex") });

    send({ type: "send", text: "shorter" });
    await turnDone();
    expect(prompts[1]).toMatch(/^<earlier_conversation>\n.*\nUser: cut it\nAssistant: Built AI - Cut\.\n<\/earlier_conversation>\n\nshorter$/);
    expect(client.state.items.map((item) => item.kind)).toEqual(["user", "assistant", "notice", "user"]);
  });

  it("compacts on request, and before a message when the user asks", async () => {
    const compacted = async (sink: TurnSink) => {
      sink.notice("Compacted.");
      sink.context({ usedTokens: 9_000, maxTokens: 1_000_000, cacheExpiresAt: null });
    };
    const { client, send, turnDone, prompts } = setup(async () => {}, compacted, compacted, async () => {});
    // Nothing to compact before the first turn.
    send({ type: "compact" });
    expect(client.state.busy).toBe(false);

    send({ type: "send", text: "one" });
    await turnDone();
    send({ type: "compact" });
    await turnDone();
    expect(client.state.context).toMatchObject({ usedTokens: 9_000 });
    send({ type: "send", text: "two", compactFirst: true });
    await turnDone();
    expect(prompts).toEqual(["one", "/compact", "/compact", "two"]);
    expect(client.state.items.map((item) => (item.kind === "user" ? item.text : item.kind))).toEqual(["one", "notice", "two", "notice"]);
  });

  it("gives a client that connects mid-turn the same state", async () => {
    const { open, opened } = gate();
    const { app, client, send, turnDone } = setup(async (sink) => {
      sink.upsert({ id: "a1", kind: "assistant", text: "partial" });
      await opened;
    });
    send({ type: "send", text: "go" });
    const late = testClient();
    app.connect(late);
    expect(late.state).toEqual(client.state);
    open();
    await turnDone();
    await late.until((state) => !state.busy);
    expect(late.state).toEqual(client.state);
  });
});

describe("app restarts", () => {
  it("keeps the thread and continues the provider's conversation", async () => {
    const store = memoryProjectStore([{ project: workspaceProject }]);
    const first = setupWith({ store }, async (sink) => {
      sink.upsert({ id: "a", kind: "assistant", text: "Built AI - Cut." });
      sink.context({ usedTokens: 40_000, maxTokens: 1_000_000, cacheExpiresAt: "2026-11-08T12:00:00.000Z" });
    });
    first.send({ type: "set_provider", provider: "claude" });
    first.send({ type: "send", text: "cut it" });
    await first.turnDone();

    const second = setupWith({ store }, async () => {});
    expect(second.client.state).toEqual({ ...first.client.state, resolve: { state: "checking" } });
    expect(second.resumeIds).toEqual(["claude-1"]);
    expect(second.dirs).toEqual(["/work"]);
    second.send({ type: "send", text: "shorter" });
    await second.turnDone();
    expect(second.prompts).toEqual(["shorter"]);

    // A new thread starts empty and isn't saved until it has a message; asking again reuses it.
    second.send({ type: "new_thread", projectId: "workspace" });
    const fresh = second.client.state.threadId;
    expect(second.client.state).toMatchObject({ items: [], context: null });
    second.send({ type: "new_thread", projectId: "workspace" });
    expect(second.client.state.threadId).toBe(fresh);
    expect([...store.threads.get("workspace")!.values()]).toMatchObject([{ title: "cut it", resumeId: "claude-1" }]);
  });

  it("saves the user's message when the turn starts, so a crash mid-turn doesn't lose it", async () => {
    const { open, opened } = gate();
    const { client, send, turnDone, store } = setup(() => opened);
    send({ type: "send", text: "cut the silences" });
    await client.until((state) => state.busy);
    await vi.waitFor(() =>
      expect([...store.threads.get("workspace")!.values()]).toMatchObject([{ items: [{ kind: "user", text: "cut the silences" }] }]),
    );
    open();
    await turnDone();
  });

  it("starts a new conversation with a summary when the earlier one can't be continued", async () => {
    const store = memoryProjectStore([
      { project: workspaceProject, threads: [savedThread({ resumeId: "gone", items: [{ id: "1", kind: "user", text: "cut it" }] })] },
    ]);
    const { client, send, turnDone, resumeIds, prompts } = setupWith(
      { store },
      async () => {
        throw new Error("No conversation found");
      },
      async (sink) => sink.upsert({ id: "a", kind: "assistant", text: "Done." }),
    );
    send({ type: "send", text: "shorter" });
    await turnDone();
    expect(resumeIds).toEqual(["gone", undefined]);
    expect(prompts[0]).toBe("shorter");
    expect(prompts[1]).toMatch(/User: cut it\n<\/earlier_conversation>\n\nshorter$/);
    expect(client.state.items.filter((item) => item.kind === "notice")).toMatchObject([
      { tone: "info", text: expect.stringContaining("Couldn't continue the earlier conversation (No conversation found)") },
    ]);
    expect(store.threads.get("workspace")!.get("thread-1")?.resumeId).toBe("claude-2");
  });
});

const EARLIER_CUT: HistoryEntry = {
  commit: "9f8e7d6",
  date: "2026-11-07T10:00:00+01:00",
  author: "Claude",
  agent: "claude",
  subject: "Earlier cut",
  prompt: "Earlier cut",
  actions: [],
  changes: [],
  thread: null,
  timelines: [],
};

describe("app edit history", () => {
  function withHistory(...scripts: Script[]) {
    const history = fakeHistory({
      record: async (turn) => {
        history.records.push(turn);
        return turn ? { commit: "abc1234", changes: ['"AI - Cut": new timeline, 3 clips, 0:10'] } : null;
      },
      recentContext: async () => "9f8e7d6 2026-11-07 10:00 Claude: Earlier cut",
      log: async () => [{ ...EARLIER_CUT, commit: `c${history.records.length}` }],
    });
    const harness = setupWith({ checkResolve: async () => connected("Doc", "doc-id"), history }, ...scripts);
    return { ...harness, records: history.records, commits: history.commits };
  }

  it("saves before every turn and after a turn that changed Resolve, with what the agent did", async () => {
    const { app, client, send, turnDone, records, prompts, store, commits } = withHistory(
      async (sink) => {
        sink.upsert({ id: "w", kind: "tool", name: "mcp__linker__create_timeline", input: { name: "Cut" }, status: "running" });
        sink.upsert({ id: "r", kind: "tool", name: "mcp__linker__list_timelines", input: {}, status: "done" });
        sink.upsert({ id: "w", status: "done" });
        sink.upsert({ id: "a", kind: "assistant", text: "Built AI - Cut." });
      },
      async (sink) => {
        sink.upsert({ id: "r2", kind: "tool", name: "mcp__linker__list_timelines", input: {}, status: "done" });
      },
    );
    await app.refreshResolve();
    send({ type: "send", text: "cut it" });
    await turnDone();
    send({ type: "send", text: "what is on it?" });
    await turnDone();

    expect(records).toEqual([
      undefined,
      {
        provider: "claude",
        prompt: "cut it",
        actions: ['create_timeline {"name":"Cut"}'],
        summary: "Built AI - Cut.",
        thread: client.state.threadId,
      },
      undefined,
    ]);
    expect(prompts[0]).toMatch(/^<edit_history>\n.*\n9f8e7d6 2026-11-07 10:00 Claude: Earlier cut\n<\/edit_history>\n\ncut it$/);
    expect(prompts[1]).toBe("what is on it?");
    // The project had no Resolve project yet: its first turn links it to the one open.
    expect(client.state.items.filter((item) => item.kind === "notice")).toMatchObject([
      { tone: "info", text: 'Linked this project to the Resolve project "Doc" in the local database "Local Database".' },
      { tone: "info", text: 'Saved to edit history (abc1234): "AI - Cut": new timeline, 3 clips, 0:10' },
    ]);
    expect(store.projects.get("workspace")).toMatchObject({
      link: { kind: "local", name: "Doc", projectId: "doc-id" },
      local: { database: { type: "Disk", name: "Local Database" } },
    });
    expect(commits).toEqual(['Link to Resolve project "Doc"']);
    expect(client.state.projects[0]).toMatchObject({ name: "Doc", open: true });
  });

  it("sends the history on request and to every client after each save", async () => {
    const { app, client, send, turnDone } = withHistory(async (sink) => {
      sink.upsert({ id: "w", kind: "tool", name: "mcp__linker__create_timeline", input: {}, status: "done" });
    });
    send({ type: "load_history" });
    await app.refreshResolve();
    send({ type: "send", text: "cut it" });
    await turnDone();
    expect(client.histories.map((entries) => entries.map((entry) => entry.commit))).toEqual([["c0"], ["c2"]]);
  });

  it.each<[string, (threadId: string) => ClientMessage]>([
    ["its thread was deleted", (threadId) => ({ type: "delete_thread", threadId })],
    ["its project was removed", () => ({ type: "remove_project", projectId: "workspace" })],
  ])("still saves a turn's Resolve changes after %s", async (_name, message) => {
    const { open, opened } = gate();
    const { app, client, send, allDone, records } = withHistory(async (sink) => {
      sink.upsert({ id: "w", kind: "tool", name: "mcp__linker__create_timeline", input: {}, status: "running" });
      await opened;
    });
    await app.refreshResolve();
    send({ type: "send", text: "cut it" });
    await client.until((state) => state.items.some((item) => item.id === "w"));
    send(message(client.state.threadId!));
    open();
    await allDone();
    expect(records.at(-1)).toMatchObject({ prompt: "cut it", actions: ["create_timeline"] });
  });
});

describe("app preview", () => {
  /** A renderer the test drives: each render waits for events the test emits. */
  function fakeRenderer() {
    const renders: { timelineId: string | null; emit: (event: PreviewEvent) => void; stopped: boolean }[] = [];
    const renderer: PreviewRenderer = {
      render(timelineId, onEvent) {
        const render = { timelineId, emit: onEvent, stopped: false };
        renders.push(render);
        return { stop: () => ((render.stopped = true), onEvent({ event: "error", message: "Preview stopped.", stopped: true })) };
      },
      file: () => null,
    };
    return { renderer, renders };
  }

  it("renders a timeline, reports progress and serves the result, outdated once Resolve is saved again", async () => {
    const { renderer, renders } = fakeRenderer();
    const { app, client, send, turnDone } = setupWith(
      {
        preview: renderer,
        checkResolve: async () => connected("Doc"),
        history: fakeHistory(),
      },
      async (sink) => {
        sink.upsert({ id: "w", kind: "tool", name: "mcp__linker__create_timeline", input: {}, status: "done" });
      },
    );
    await app.refreshResolve();
    send({ type: "render_preview", timelineId: "t1" });
    expect(client.state.preview).toEqual({ state: "rendering", timeline: null, percent: 0 });

    // Resolve does one thing at a time: no turn while a render runs, and no second render.
    send({ type: "send", text: "cut it" });
    send({ type: "render_preview" });
    expect(client.state.items).toEqual([]);
    expect(renders).toHaveLength(1);

    renders[0]!.emit({ event: "start", timeline: { id: "t1", name: "AI - Cut" } });
    renders[0]!.emit({ event: "progress", percent: 40 });
    expect(client.state.preview).toEqual({ state: "rendering", timeline: { id: "t1", name: "AI - Cut" }, percent: 40 });
    renders[0]!.emit({ event: "done", file: "/previews/preview-1.mp4" });
    expect(client.state.preview).toMatchObject({ state: "ready", url: "/preview/preview-1.mp4", outdated: false });

    send({ type: "send", text: "cut it" });
    await turnDone();
    expect(client.state.preview).toMatchObject({ state: "ready", outdated: true });
  });

  it("stops a render and shows failures", () => {
    const { renderer, renders } = fakeRenderer();
    const { client, send } = setupWith({ preview: renderer });
    send({ type: "render_preview" });
    send({ type: "stop_preview" });
    expect(renders[0]).toMatchObject({ timelineId: null, stopped: true });
    expect(client.state.preview).toEqual({ state: "idle" });

    send({ type: "render_preview" });
    renders[1]!.emit({ event: "error", message: "Resolve has no timeline open." });
    expect(client.state.preview).toEqual({ state: "error", message: "Resolve has no timeline open." });
  });
});

describe("app projects", () => {
  const linked = (name: string, projectId: string | null, fields: Partial<SavedProject> = {}): SavedProject => ({
    id: name.toLowerCase(),
    dir: `/projects/${name}`,
    name: null,
    link: { kind: "local", name, projectId },
    local: {},
    icon: null,
    pinned: false,
    archived: false,
    ...fields,
  });

  /** Resolve's status, changed by the test as the user switches projects in Resolve. */
  function resolveWith(status: ResolveStatus) {
    const current = { status };
    return { current, checkResolve: async () => current.status };
  }

  it("shows a preview only in its own project's threads", () => {
    const preview: PreviewRenderer = { render: () => ({ stop() {} }), file: () => null };
    const { client, send } = setupWith({
      store: memoryProjectStore([{ project: linked("Interview", null) }, { project: linked("Promo", null) }]),
      preview,
    });
    expect(client.state.projectId).toBe("interview");
    send({ type: "render_preview" });
    expect(client.state.preview).toMatchObject({ state: "rendering" });
    send({ type: "new_thread", projectId: "promo" });
    expect(client.state.preview).toEqual({ state: "idle" });
    expect(client.state.blocked?.message).toMatch(/rendering a preview/);
  });

  it("doesn't preview another Resolve project as this project's", async () => {
    const renders: (string | null)[] = [];
    const preview: PreviewRenderer = { render: (id) => (renders.push(id), { stop() {} }), file: () => null };
    const { app, client, send } = setupWith({
      store: memoryProjectStore([{ project: linked("Interview", "int-id") }]),
      checkResolve: async () => connected("Promo", "promo-id"),
      history: fakeHistory({ timeline: async () => ({ id: "t1" }) as PlayableTimeline }),
      preview,
    });
    await app.refreshResolve();
    send({ type: "render_preview", timelineId: "t1" });
    expect(renders).toEqual([]);
    expect(client.toasts).toEqual([expect.stringContaining('Resolve has "Promo" open')]);
    await expect(app.playableTimeline("interview", null, null)).rejects.toThrow('Resolve has "Promo" open');
    // A saved version comes from the edit history, not from Resolve.
    expect(await app.playableTimeline("interview", "t1", "abc1234")).toEqual({ id: "t1" });
  });

  it("refuses a turn while Resolve has another project open, and runs it once the project is open", async () => {
    const resolve = resolveWith(connected("Promo", "promo-id"));
    const { app, client, send, turnDone, prompts } = setupWith(
      { store: memoryProjectStore([{ project: linked("Interview", "int-id") }]), checkResolve: resolve.checkResolve },
      async () => {},
    );
    await app.refreshResolve();
    expect(client.state.blocked).toEqual({
      message: 'Resolve has "Promo" open, not this project\'s "Interview". Open "Interview" in Resolve to continue.',
      canOpen: false,
    });
    expect(client.state.projects[0]).toMatchObject({ name: "Interview", open: false });

    send({ type: "send", text: "cut it" });
    await client.until(() => client.refused.length > 0);
    expect(client.refused).toEqual(["cut it"]);
    expect(client.toasts).toEqual([client.state.blocked!.message]);
    expect(prompts).toEqual([]);
    expect(client.state.items).toEqual([]);

    resolve.current.status = connected("Interview", "int-id");
    send({ type: "send", text: "cut it" });
    await client.until((state) => state.items.length > 0);
    await turnDone();
    expect(prompts).toEqual(["cut it"]);
    expect(client.state.blocked).toBeNull();
  });

  it("won't link a new project to a Resolve project another project has", async () => {
    const store = memoryProjectStore([{ project: linked("Doc", "doc-id") }, { project: { ...workspaceProject } }]);
    const { app, client, send } = setupWith({ store, checkResolve: async () => connected("Doc", "doc-id") });
    await app.refreshResolve();
    expect(client.state.projectId).toBe("doc");
    send({ type: "new_thread", projectId: "workspace" });
    expect(client.state.blocked?.message).toBe(
      'Resolve has "Doc" open, which belongs to the project "Doc". Switch to that project, or open another one in Resolve.',
    );
    send({ type: "send", text: "cut it" });
    await client.until(() => client.refused.length > 0);
    expect(store.projects.get("workspace")?.link).toBeNull();
  });

  it("lets one thread work in Resolve at a time", async () => {
    const { open, opened } = gate();
    const { client, send, allDone, prompts } = setup(
      () => opened,
      async () => {},
    );
    send({ type: "send", text: "first" });
    await client.until((state) => state.items.length > 0);
    send({ type: "new_thread", projectId: "workspace" });
    expect(client.state.blocked).toEqual({ message: 'Claude is working in "first". One thread works in Resolve at a time.', canOpen: false });
    send({ type: "send", text: "second" });
    expect(client.refused).toEqual(["second"]);
    open();
    await allDone();
    send({ type: "send", text: "second" });
    await allDone();
    expect(prompts).toEqual(["first", "second"]);
  });

  it("learns the project's id when it is open, and follows a rename in Resolve", async () => {
    const resolve = resolveWith(connected("Doc", "doc-id"));
    const history = fakeHistory();
    const store = memoryProjectStore([{ project: linked("Doc", null) }]);
    const { app, client } = setupWith({ store, history, checkResolve: resolve.checkResolve });
    await app.refreshResolve();
    expect(store.projects.get("doc")?.link).toEqual({ kind: "local", name: "Doc", projectId: "doc-id" });

    resolve.current.status = connected("Doc v2", "doc-id");
    await app.refreshResolve();
    expect(store.projects.get("doc")?.link).toEqual({ kind: "local", name: "Doc v2", projectId: "doc-id" });
    expect(history.commits).toEqual(['Link to Resolve project "Doc"', 'Resolve project renamed to "Doc v2"']);
    expect(client.state.projects[0]).toMatchObject({ name: "Doc v2", open: true });
  });

  it("adds projects from Resolve, its database and Blackmagic Cloud, once each", async () => {
    const store = memoryProjectStore([{ project: { ...workspaceProject } }]);
    const { app, client, send } = setupWith({ store, checkResolve: async () => connected("Promo", "promo-id") });
    await app.refreshResolve();

    send({ type: "add_project", source: { kind: "open" } });
    expect(client.state.projectId).toBe("project-1");
    expect(client.state.projects[1]).toMatchObject({
      name: "Promo",
      link: { kind: "local", name: "Promo", projectId: "promo-id" },
      local: { database: { type: "Disk", name: "Local Database" } },
      open: true,
    });
    send({ type: "add_project", source: { kind: "open" } });
    expect(client.toasts).toEqual(['"Promo" is already in Linker-Edit.']);

    send({ type: "add_project", source: { kind: "cloud", name: " " } });
    expect(client.toasts.at(-1)).toBe("A cloud project needs its name.");
    send({ type: "add_project", source: { kind: "cloud", name: "Client X" } });
    send({
      type: "add_project",
      source: { kind: "library", name: "Old", folder: ["2025"], database: { type: "PostgreSQL", name: "Team", host: "10.0.0.5" } },
    });
    expect(client.state.projects.slice(2)).toMatchObject([
      { name: "Client X", link: { kind: "cloud", projectId: null }, local: {} },
      { name: "Old", link: { kind: "network" }, local: { folder: ["2025"], database: { name: "Team" } } },
    ]);
    expect(store.projects.size).toBe(4);
  });

  describe("a turn that starts Resolve", () => {
    function closedResolve(project: SavedProject, opensAs: ResolveStatus) {
      const resolve = resolveWith({ state: "not_running" });
      const opens: unknown[] = [];
      const ran: string[] = [];
      const results: { text: string; isError: boolean }[] = [];
      const history = fakeHistory();
      let call: App["resolveTool"] = () => Promise.reject(new Error("No app yet"));
      const harness = setupWith(
        {
          store: memoryProjectStore([{ project }]),
          checkResolve: resolve.checkResolve,
          history,
          resolveTools: async (tool) => (ran.push(tool), { text: tool }),
          backupProject: async () => "backup.drp",
          resolveProjects: {
            library: async () => ({ database: null, databases: [], projects: [] }),
            start: async () => {
              resolve.current.status = opensAs;
              return true;
            },
            open: async (link, _local, options) => {
              opens.push([link.name, options?.onlyIfIdle]);
              resolve.current.status = opensAs;
              return { project: { name: link.name, id: "int-id" }, database: null, folder: null };
            },
          },
        },
        async () => {
          for (const tool of ["create_timeline", "start_resolve", "create_timeline"]) results.push(await call(tool, {}));
        },
      );
      call = harness.app.resolveTool;
      return { ...harness, opens, ran, results, history };
    }

    it("opens the thread's own project, saves the user's changes and goes on, without asking in supervised mode", async () => {
      const { send, client, turnDone, opens, ran, results, history } = closedResolve(linked("Interview", "int-id"), connected("Interview", "int-id"));
      send({ type: "set_approval_mode", mode: "ask" });
      send({ type: "send", text: "cut it" });
      await client.until((state) => state.approvals.length > 0);
      expect(client.state.approvals).toMatchObject([{ tool: "mcp__linker__create_timeline" }]);
      expect(history.records).toEqual([undefined]);
      send({ type: "approve", id: client.state.approvals[0]!.id, allow: true });
      await turnDone();
      expect(opens).toEqual([["Interview", true]]);
      expect(results).toEqual([
        { text: "Resolve isn't running. Start it with start_resolve.", isError: true },
        { text: 'Resolve is running with "Interview" open.', isError: false },
        { text: "create_timeline", isError: false },
      ]);
      expect(ran).toEqual(["create_timeline"]);
    });

    it("changes nothing when Resolve ends up with another project, or the folder has none yet", async () => {
      const other = closedResolve(linked("Interview", "int-id"), connected("Promo", "promo-id"));
      other.send({ type: "send", text: "cut it" });
      await other.turnDone();
      expect(other.results.slice(1).map((result) => result.text)).toEqual(Array(2).fill(expect.stringContaining('Resolve has "Promo" open')));

      const unlinked = closedResolve({ ...workspaceProject }, connected("Promo", "promo-id"));
      unlinked.send({ type: "send", text: "cut it" });
      await unlinked.turnDone();
      expect(unlinked.opens).toEqual([]);
      expect(unlinked.results.at(-1)).toMatchObject({ isError: true, text: expect.stringContaining("isn't linked") });
      expect([...other.ran, ...unlinked.ran]).toEqual([]);
    });
  });

  describe("with another project open in Resolve", () => {
    /** Interview's thread while Resolve has Promo open; what Resolve has moves with what Linker-Edit opens. */
    function twoProjects(projects: SavedProject[], script: (call: App["resolveTool"], sink: TurnSink) => Promise<void> = async () => {}) {
      const ids: Record<string, string> = { Interview: "int-id", Promo: "promo-id", Doc: "doc-id" };
      const current: { status: ResolveCheck } = { status: { ...connected("Promo", "promo-id"), activity: "a" } };
      const opens: string[] = [];
      const ran: string[] = [];
      const history = fakeHistory();
      let call: App["resolveTool"] = () => Promise.reject(new Error("No app yet"));
      const open = (name: string) => (current.status = { ...connected(name, ids[name]!), activity: "a" });
      const harness = setupWith(
        {
          store: memoryProjectStore(projects.map((project) => ({ project }))),
          checkResolve: async () => current.status,
          history,
          resolveTools: async (tool) => (ran.push(`${tool} in ${(current.status as { project?: string }).project}`), { text: tool }),
          backupProject: async () => "backup.drp",
          resolveProjects: {
            library: async () => ({ database: null, databases: [], projects: [] }),
            start: async () => false,
            open: async (link) => {
              opens.push(link.name);
              open(link.name);
              return { project: { name: link.name, id: ids[link.name]! }, database: { type: "Disk", name: "Local Database" }, folder: null };
            },
          },
        },
        ...Array.from({ length: 3 }, (): Script => (sink) => script(call, sink)),
      );
      call = harness.app.resolveTool;
      return { ...harness, current, open, opens, ran, history };
    }

    const interview = linked("Interview", "int-id");
    const promo = linked("Promo", "promo-id");

    it("asks before leaving one the user may be working in, with full access too, and stays when they say no", async () => {
      const { app, client, send, turnDone, opens, prompts } = twoProjects([interview]);
      await app.refreshResolve();
      expect(client.state.blocked).toMatchObject({ canSend: true, canOpen: true });
      send({ type: "send", text: "cut it" });
      await client.until((state) => state.approvals.length > 0);
      expect(client.state.approvals[0]).toMatchObject({ tool: "mcp__linker__switch_project", input: { open: "Promo", wants: "Interview", why: expect.stringContaining("may be working") } });
      // Full access doesn't answer it.
      send({ type: "set_approval_mode", mode: "ask" });
      send({ type: "set_approval_mode", mode: "auto" });
      expect(client.state.approvals).toHaveLength(1);
      send({ type: "approve", id: client.state.approvals[0]!.id, allow: false });
      await client.until(() => client.refused.length > 0);
      expect(client.refused).toEqual(["cut it"]);
      expect([opens, prompts]).toEqual([[], []]);

      send({ type: "send", text: "cut it" });
      await client.until((state) => state.approvals.length > 0);
      send({ type: "approve", id: client.state.approvals[0]!.id, allow: true });
      await client.until((state) => state.items.length > 0);
      await turnDone();
      expect(prompts).toEqual(["cut it"]);
      expect(client.state.items.filter((item) => item.kind === "notice")[0]).toMatchObject({ text: 'Saved "Promo" and opened "Interview" in Resolve.' });

      // And back with one click.
      expect(client.state.reopen).toBe("Promo");
      await app.refreshResolve();
      send({ type: "reopen_previous" });
      await client.until((state) => state.reopen === null && state.resolve.state === "connected" && state.resolve.project === "Promo");
      expect(opens).toEqual(["Interview", "Promo"]);
    });

    it("switches without asking when nothing moved for a while, and still asks in supervised mode", async () => {
      const now = vi.spyOn(Date, "now");
      try {
        const { app, client, send, turnDone, open, opens } = twoProjects([interview]);
        await app.refreshResolve();
        now.mockReturnValue(new Date().getTime() + QUIET_MS);
        send({ type: "send", text: "cut it" });
        await client.until((state) => state.items.length > 0);
        await turnDone();
        expect(opens).toEqual(["Interview"]);

        open("Promo");
        await app.refreshResolve();
        now.mockReturnValue(new Date().getTime() + 3 * QUIET_MS);
        send({ type: "set_approval_mode", mode: "ask" });
        send({ type: "send", text: "again" });
        await client.until((state) => state.approvals.length > 0);
        send({ type: "stop" });
        await client.until(() => client.refused.length > 0);
        expect(opens).toEqual(["Interview"]);
      } finally {
        now.mockRestore();
      }
    });

    it("visits another project read only and is back in its own before the turn's save", async () => {
      const results: { text: string; isError: boolean }[] = [];
      let backBeforeSave = "";
      const { app, client, send, turnDone, open, opens, ran, history } = twoProjects([interview, promo], async (call, sink) => {
        // A change, so the turn ends with a save.
        sink.upsert({ id: "t1", kind: "tool", name: "mcp__linker__create_timeline", input: { name: "X" }, status: "done" });
        for (const [tool, input] of [
          ["list_projects", {}],
          ["visit_project", { project: "Nope" }],
          ["visit_project", { project: "Promo" }],
          ["list_timelines", {}],
          ["create_timeline", { name: "X" }],
          ["set_current_timeline", {}],
          ["start_resolve", {}],
          ["end_visit", {}],
          ["list_timelines", {}],
          ["visit_project", { project: "Promo" }],
        ] as const) results.push(await call(tool, input));
      });
      open("Interview");
      await app.refreshResolve();
      const record = history.record;
      history.record = (turn) => ((backBeforeSave = opens.at(-1) ?? ""), record(turn));
      send({ type: "send", text: "same intro as Promo" });
      await client.until((state) => state.items.length > 0);
      await turnDone();
      expect(JSON.parse(results[0]!.text)).toEqual({ projects: [{ name: "Interview", open: true }, { name: "Promo" }] });
      expect(results.map((result) => result.isError)).toEqual([false, true, false, false, true, true, true, false, false, false]);
      expect(ran).toEqual(["list_timelines in Promo", "list_timelines in Interview"]);
      // The last visit was left open by the agent.
      expect(opens).toEqual(["Promo", "Interview", "Promo", "Interview"]);
      expect(backBeforeSave).toBe("Interview");
      expect(client.state.resolve).toMatchObject({ project: "Interview" });
    });

    it("lets a skill thread look, asking before it leaves what the user is working in, and goes back to that", async () => {
      const results: { text: string; isError: boolean }[] = [];
      const skill: SavedProject = { ...workspaceProject, id: "skill", setup: true, collection: { id: "c", name: "Ferret" } };
      const { app, client, send, turnDone, opens, ran } = twoProjects([interview, promo, skill], async (call) => {
        for (const [tool, input] of [
          ["list_timelines", {}],
          ["visit_project", { project: "Interview" }],
          ["list_timelines", {}],
          ["create_timeline", { name: "X" }],
        ] as const) results.push(await call(tool, input));
      });
      await app.refreshResolve();
      send({ type: "new_thread", projectId: "skill" });
      send({ type: "send", text: "learn from Interview" });
      await client.until((state) => state.approvals.length > 0);
      expect(client.state.approvals[0]!.input).toMatchObject({ open: "Promo", wants: expect.stringContaining("Interview") });
      send({ type: "approve", id: client.state.approvals[0]!.id, allow: true });
      await turnDone();
      expect(results.map((result) => result.isError)).toEqual([true, false, false, true]);
      expect(ran).toEqual(["list_timelines in Interview"]);
      expect(opens).toEqual(["Interview", "Promo"]);
    });

    it("asks before it leaves what the user is working in when the visits start there", async () => {
      const results: boolean[] = [];
      const skill: SavedProject = { ...workspaceProject, id: "skill", setup: true, collection: { id: "c", name: "Ferret" } };
      const { app, client, send, turnDone, opens } = twoProjects([interview, promo, linked("Doc", "doc-id"), skill], async (call) => {
        for (const project of ["Promo", "Interview", "Doc"]) results.push((await call("visit_project", { project })).isError);
      });
      await app.refreshResolve();
      send({ type: "new_thread", projectId: "skill" });
      send({ type: "send", text: "learn from all three" });
      await client.until((state) => state.approvals.length > 0);
      expect(client.state.approvals[0]!.input).toMatchObject({ open: "Promo", wants: expect.stringContaining("Interview") });
      send({ type: "approve", id: client.state.approvals[0]!.id, allow: true });
      // From Interview, which the visit opened, to Doc without another question.
      await turnDone();
      expect(results).toEqual([false, false, false]);
      expect(opens).toEqual(["Interview", "Doc", "Promo"]);
    });

    it("comes back when the user stops the turn", async () => {
      let visited = () => {};
      const visiting = new Promise<void>((resolve) => (visited = resolve));
      const { app, send, client, turnDone, open, opens } = twoProjects([interview, promo], async (call) => {
        await call("visit_project", { project: "Promo" });
        visited();
      });
      open("Interview");
      await app.refreshResolve();
      send({ type: "send", text: "look at Promo" });
      await visiting;
      send({ type: "stop" });
      await turnDone();
      expect(opens).toEqual(["Promo", "Interview"]);
      expect(client.state.resolve).toMatchObject({ project: "Interview" });
    });

    it("doesn't visit from or to a Blackmagic Cloud project, which no script may open", async () => {
      const results: { text: string; isError: boolean }[] = [];
      const cloud = { ...linked("Doc", "doc-id"), link: { kind: "cloud", name: "Doc", projectId: "doc-id" } } as SavedProject;
      const visit = (project: string) => async (call: App["resolveTool"]) => void results.push(await call("visit_project", { project }));
      const to = twoProjects([interview, cloud], visit("Doc"));
      to.open("Interview");
      await to.app.refreshResolve();
      to.send({ type: "send", text: "look" });
      await to.client.until((state) => state.items.length > 0);
      await to.turnDone();
      const from = twoProjects([cloud, promo], visit("Promo"));
      from.open("Doc");
      await from.app.refreshResolve();
      from.send({ type: "send", text: "look" });
      await from.client.until((state) => state.items.length > 0);
      await from.turnDone();
      expect(results).toEqual(Array(2).fill({ text: expect.stringContaining("Blackmagic Cloud"), isError: true }));
      expect([...to.opens, ...from.opens]).toEqual([]);
    });
  });

  it("opens a project in Resolve, and says when it couldn't", async () => {
    const resolve = resolveWith(connected("Promo", "promo-id"));
    const opens: string[] = [];
    let fail = false;
    const resolveProjects: AppOptions["resolveProjects"] = {
      library: async () => ({ database: null, databases: [], projects: [] }),
      start: async () => false,
      open: async (link) => {
        opens.push(link.name);
        if (fail) throw new Error("Resolve couldn't find or open the project.");
        resolve.current.status = connected("Interview", "int-id");
        return { project: { name: "Interview", id: "int-id" }, database: { type: "Disk", name: "Local Database" }, folder: ["Clients"] };
      },
    };
    const store = memoryProjectStore([{ project: linked("Interview", null) }]);
    const { app, client, send } = setupWith({ store, checkResolve: resolve.checkResolve, resolveProjects });
    await app.refreshResolve();
    expect(client.state.blocked?.canOpen).toBe(true);

    fail = true;
    send({ type: "open_in_resolve", projectId: "interview" });
    expect(client.state.opening).toBe("interview");
    await client.until((state) => state.opening === null);
    expect(client.toasts).toEqual(['Couldn\'t open "Interview" in Resolve: Resolve couldn\'t find or open the project.']);

    fail = false;
    send({ type: "open_in_resolve", projectId: "interview" });
    await client.until((state) => state.opening === null && state.blocked === null);
    expect(opens).toEqual(["Interview", "Interview"]);
    expect(store.projects.get("interview")).toMatchObject({
      link: { projectId: "int-id" },
      local: { folder: ["Clients"], database: { name: "Local Database" } },
    });
  });

  it("renames, archives, unarchives and deletes threads", async () => {
    const store = memoryProjectStore([
      { project: { ...workspaceProject }, threads: [savedThread({ items: [{ id: "1", kind: "user", text: "cut it" }] })] },
    ]);
    const { client, send } = setupWith({ store });
    expect(client.state.threadId).toBe("thread-1");
    const summary = () => client.state.projects[0]!.threads.find((thread) => thread.id === "thread-1");

    send({ type: "rename_thread", threadId: "thread-1", title: "  Rough cut  " });
    expect(summary()).toMatchObject({ title: "Rough cut", archived: false });
    send({ type: "archive_thread", threadId: "thread-1", archived: true });
    expect(summary()).toMatchObject({ archived: true });
    expect(client.state.threadId).not.toBe("thread-1");
    expect(store.threads.get("workspace")!.get("thread-1")).toMatchObject({ title: "Rough cut", archivedAt: expect.any(String) });

    send({ type: "archive_thread", threadId: "thread-1", archived: false });
    expect(summary()).toMatchObject({ archived: false });
    // Archiving a thread with nothing in it just drops it.
    const empty = client.state.threadId!;
    send({ type: "archive_thread", threadId: empty, archived: true });
    expect(client.state.projects[0]!.threads.map((thread) => thread.id)).not.toContain(empty);

    send({ type: "delete_thread", threadId: "thread-1" });
    expect(summary()).toBeUndefined();
    expect(store.threads.get("workspace")!.size).toBe(0);
  });

  it("removes a project from Linker-Edit only, moving its viewers to another project", async () => {
    const store = memoryProjectStore([{ project: linked("Doc", "doc-id") }, { project: { ...workspaceProject } }]);
    const { client, send } = setupWith({ store });
    expect(client.state.projectId).toBe("doc");
    send({ type: "remove_project", projectId: "doc" });
    expect(client.state.projects.map((project) => project.id)).toEqual(["workspace"]);
    expect(client.state.projectId).toBe("workspace");
    expect([...store.projects.keys()]).toEqual(["workspace"]);
  });
});

describe("app sidebar (T3 Code)", () => {
  it("pins, settles, snoozes and reads threads, and keeps it all across a restart", async () => {
    const store = memoryProjectStore([{ project: workspaceProject }]);
    const { client, send, turnDone } = setupWith({ store }, async (sink) => sink.upsert({ id: "a", kind: "assistant", text: "Cut." }));
    send({ type: "send", text: "cut it" });
    await turnDone();
    const threadId = client.state.threadId!;
    const summary = () => client.state.projects[0]!.threads.find((thread) => thread.id === threadId)!;
    expect(summary()).toMatchObject({ pinnedAt: null, settledAt: null, lastReadAt: null });
    expect(summary().lastSentAt).toEqual(expect.any(String));
    expect(summary().turnEndedAt).toEqual(expect.any(String));

    send({ type: "pin_thread", threadId, pinned: true });
    expect(summary().pinnedAt).toEqual(expect.any(String));

    // Settling unpins and wakes it, as in T3 Code.
    send({ type: "snooze_thread", threadId, until: new Date(Date.now() + 3_600_000).toISOString() });
    expect(summary().snoozedUntil).toEqual(expect.any(String));
    send({ type: "settle_thread", threadId, settled: true });
    expect(summary()).toMatchObject({ pinnedAt: null, snoozedUntil: null, settledAt: expect.any(String), unsettledAt: null });

    // A snooze time in the past is ignored.
    send({ type: "snooze_thread", threadId, until: "2020-01-01T00:00:00.000Z" });
    expect(summary().snoozedUntil).toBeNull();

    send({ type: "mark_read", threadId, read: true });
    const read = summary().lastReadAt!;
    expect(Date.parse(read)).toBeGreaterThanOrEqual(Date.parse(summary().turnEndedAt!));
    send({ type: "mark_read", threadId, read: false });
    expect(Date.parse(summary().lastReadAt!)).toBeLessThan(Date.parse(summary().turnEndedAt!));

    const saved = [...store.threads.get("workspace")!.values()][0]!;
    expect(saved).toMatchObject({ settledAt: summary().settledAt, lastReadAt: summary().lastReadAt, pinnedAt: null });
    const again = setupWith({ store });
    expect(again.client.state.projects[0]!.threads.find((thread) => thread.id === threadId)).toMatchObject({
      settledAt: summary().settledAt,
      lastReadAt: summary().lastReadAt,
    });
  });

  it("brings a settled thread back when the user writes in it", async () => {
    const { client, send, turnDone } = setup(
      async () => {},
      async () => {},
    );
    send({ type: "send", text: "first" });
    await turnDone();
    const threadId = client.state.threadId!;
    send({ type: "settle_thread", threadId, settled: true });
    send({ type: "send", text: "one more thing" });
    await turnDone();
    expect(client.state.projects[0]!.threads[0]).toMatchObject({ settledAt: null, unsettledAt: expect.any(String) });
  });

  it("saves a project's icon in its project file, and its pin and archive on this machine", () => {
    const store = memoryProjectStore([{ project: workspaceProject }]);
    const { client, send } = setupWith({ store });
    send({ type: "update_project", projectId: "workspace", icon: { kind: "lucide", name: "rocket", color: "violet" }, pinned: true });
    expect(client.state.projects[0]).toMatchObject({ icon: { kind: "lucide", name: "rocket", color: "violet" }, pinned: true, archived: false });
    // A malformed icon falls back to the monogram.
    send({ type: "update_project", projectId: "workspace", icon: { kind: "lucide", name: "Not An Icon", color: "violet" } as never, archived: true });
    expect(client.state.projects[0]).toMatchObject({ icon: null, pinned: true, archived: true });
    expect(store.projects.get("workspace")).toMatchObject({ icon: null, pinned: true, archived: true });
  });
});

describe("meta broadcasts", () => {
  /** A client that records what it gets, raw. */
  function recorder(metaPatches: boolean) {
    const messages: ServerMessage[] = [];
    return { messages, metaPatches, send: (data: string) => void messages.push(JSON.parse(data) as ServerMessage), types: () => messages.map((message) => message.type) };
  }

  it("sends a meta only when it changed, and only the changed fields to clients that take patches", async () => {
    const usage = { usedTokens: 1200, maxTokens: 200000, cacheExpiresAt: null };
    const { app, send, turnDone } = setup(async (sink) => {
      sink.context(usage);
      // The same usage again changes nothing.
      sink.context(usage);
      sink.upsert({ id: "a1", kind: "assistant", text: "Done" });
    });
    const full = recorder(false);
    const patched = recorder(true);
    app.connect(full);
    app.connect(patched);
    expect(full.types()).toEqual(["snapshot"]);
    expect(patched.types()).toEqual(["snapshot"]);

    send({ type: "set_approval_mode", mode: "ask" });
    send({ type: "set_approval_mode", mode: "ask" });
    expect(full.types()).toEqual(["snapshot", "meta"]);
    expect(full.messages[1]).toMatchObject({ type: "meta", meta: { approvalMode: "ask", projects: [{ id: "workspace" }] } });
    expect(patched.messages.slice(1)).toEqual([{ type: "meta_patch", meta: { approvalMode: "ask" } }]);

    send({ type: "send", text: "cut it" });
    await turnDone();
    const patches = patched.messages.filter((message) => message.type === "meta_patch").map((message) => message.meta);
    expect(patches.filter((meta) => "context" in meta)).toEqual([{ context: usage }]);
    // Each meta the full client got differs from the one before.
    const metas = full.messages.filter((message) => message.type === "meta").map((message) => JSON.stringify(message.meta));
    expect(new Set(metas).size).toBe(metas.length);
    // Both end with the same state.
    const fold = (messages: ServerMessage[]) => messages.reduce<AppState | null>((state, message) => applyServerMessage(state, message), null);
    expect(fold(patched.messages)).toEqual(fold(full.messages));
  });
});
