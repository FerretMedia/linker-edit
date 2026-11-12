import { describe, expect, it } from "vitest";
import { applyServerMessage } from "../src/lib/appState.ts";
import type { AppState, ClientMessage, HistoryEntry, Provider, ResolveStatus, ServerMessage } from "../shared/protocol.ts";
import { createApp, type App, type AppOptions } from "./app.ts";
import type { AgentTurn, History } from "./history/history.ts";
import type { SavedProject, SavedThread } from "./projects/store.ts";
import { memoryProjectStore, workspaceProject } from "./projects/testing.ts";
import type { PreviewEvent, PreviewRenderer } from "./preview/preview.ts";
import type { ProviderSession, TurnSink } from "./providers/types.ts";

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

type HarnessOptions = Partial<Pick<AppOptions, "checkResolve" | "preview" | "resolveProjects" | "resolveTools">> & {
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
  });
  const client = testClient();
  const disconnect = app.connect(client);
  const send = (message: ClientMessage) => app.handleMessage(client, message);
  const turnDone = () => client.until((state) => !state.busy);
  /** Every turn is over, including one in a thread the client moved away from or deleted. */
  const allDone = () => client.until(() => !app.busy());
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
    ...fields,
  });

  /** Resolve's status, changed by the test as the user switches projects in Resolve. */
  function resolveWith(status: ResolveStatus) {
    const current = { status };
    return { current, checkResolve: async () => current.status };
  }

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

    send({ type: "add_project", source: { kind: "cloud", name: "Client X", mediaPath: " ", syncMode: "proxy_only" } });
    expect(client.toasts.at(-1)).toBe("A cloud project needs its name and a media folder.");
    send({ type: "add_project", source: { kind: "cloud", name: "Client X", mediaPath: "D:\\Cloud\\Client X", syncMode: "proxy_only" } });
    send({
      type: "add_project",
      source: { kind: "library", name: "Old", folder: ["2025"], database: { type: "PostgreSQL", name: "Team", host: "10.0.0.5" } },
    });
    expect(client.state.projects.slice(2)).toMatchObject([
      { name: "Client X", link: { kind: "cloud", projectId: null }, local: { mediaPath: "D:\\Cloud\\Client X", syncMode: "proxy_only" } },
      { name: "Old", link: { kind: "network" }, local: { folder: ["2025"], database: { name: "Team" } } },
    ]);
    expect(store.projects.size).toBe(4);
  });

  it("opens a project in Resolve, and says when it couldn't", async () => {
    const resolve = resolveWith(connected("Promo", "promo-id"));
    const opens: string[] = [];
    let fail = false;
    const resolveProjects: AppOptions["resolveProjects"] = {
      library: async () => ({ database: null, databases: [], projects: [] }),
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
