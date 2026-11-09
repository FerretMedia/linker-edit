import { describe, expect, it } from "vitest";
import { applyServerMessage } from "../src/lib/appState.ts";
import type { AppState, ClientMessage, HistoryEntry, Provider, ServerMessage } from "../shared/protocol.ts";
import { createApp, type AppOptions } from "./app.ts";
import type { AgentTurn } from "./history/history.ts";
import type { PreviewEvent, PreviewRenderer } from "./preview/preview.ts";
import type { ProviderSession, TurnSink } from "./providers/types.ts";

type Script = (sink: TurnSink, signal: AbortSignal) => Promise<void>;

/** A client that keeps state the way the UI does, and resolves waits as messages arrive. */
function testClient() {
  let state: AppState | null = null;
  const histories: HistoryEntry[][] = [];
  const waiters: { check: () => boolean; resolve: () => void }[] = [];
  return {
    histories,
    send(data: string) {
      const message = JSON.parse(data) as ServerMessage;
      if (message.type === "history") histories.push(message.entries);
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

function setupWith(options: Partial<Pick<AppOptions, "history" | "checkResolve" | "preview">>, ...scripts: Script[]) {
  const sessions: Provider[] = [];
  const prompts: string[] = [];
  const app = createApp({
    version: "0.0.0",
    workspaceDir: "/work",
    createSession: (provider): ProviderSession => {
      sessions.push(provider);
      return {
        send: (text, sink, signal) => {
          prompts.push(text);
          const script = scripts.shift();
          if (!script) throw new Error("No script left for this send");
          return script(sink, signal);
        },
      };
    },
    checkResolve: options.checkResolve ?? (async () => ({ state: "not_running" })),
    history: options.history,
    preview: options.preview,
  });
  const client = testClient();
  app.connect(client);
  const send = (message: ClientMessage) => app.handleMessage(message);
  const turnDone = () => client.until((state) => !state.busy);
  return { app, client, send, turnDone, sessions, prompts };
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
  function awaitingApproval() {
    const answers: boolean[] = [];
    const harness = setup(async (sink) => {
      answers.push(await sink.requestApproval("mcp__resolve__timeline", { action: "create_empty" }));
    });
    harness.send({ type: "send", text: "build it" });
    return { ...harness, answers };
  }

  it.each([true, false])("passes the user's answer (%s) to the provider", async (allow) => {
    const { client, send, turnDone, answers } = awaitingApproval();
    const [request] = client.state.approvals;
    expect(request).toMatchObject({ tool: "mcp__resolve__timeline", input: { action: "create_empty" } });
    send({ type: "approve", id: request!.id, allow });
    await turnDone();
    expect(answers).toEqual([allow]);
    expect(client.state.approvals).toEqual([]);
  });

  it("allows pending approvals when switched to auto", async () => {
    const { send, turnDone, answers } = awaitingApproval();
    send({ type: "set_approval_mode", mode: "auto" });
    await turnDone();
    expect(answers).toEqual([true]);
  });

  it.each<ClientMessage>([{ type: "stop" }, { type: "new_chat" }, { type: "set_provider", provider: "codex" }])(
    "denies pending approvals on %j",
    async (message) => {
      const { client, send, turnDone, answers } = awaitingApproval();
      send(message);
      expect(client.state.approvals).toEqual([]);
      await turnDone();
      expect(answers).toEqual([false]);
    },
  );
});

describe("app chats", () => {
  it("keeps a turn that outlives its chat out of the next one", async () => {
    const answers: boolean[] = [];
    const { client, send, turnDone } = setup(async (sink) => {
      answers.push(await sink.requestApproval("Write", {}));
      sink.upsert({ id: "late", kind: "assistant", text: "late reply" });
      sink.notice("late notice");
      answers.push(await sink.requestApproval("Write", {}));
    });
    send({ type: "send", text: "go" });
    send({ type: "new_chat" });
    await turnDone();
    expect(answers).toEqual([false, false]);
    expect(client.state.items).toEqual([]);
    expect(client.state.approvals).toEqual([]);
  });

  it("starts a fresh session when the provider changes, and only then", async () => {
    const { client, send, sessions } = setup();
    send({ type: "set_provider", provider: "claude" });
    send({ type: "set_provider", provider: "codex" });
    expect(sessions).toEqual(["claude", "codex"]);
    expect(client.state.provider).toBe("codex");
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

const EARLIER_CUT: HistoryEntry = {
  commit: "9f8e7d6",
  date: "2026-11-07T10:00:00+01:00",
  author: "Claude",
  agent: "claude",
  subject: "Earlier cut",
  prompt: "Earlier cut",
  actions: [],
  changes: [],
  timelines: [],
};

describe("app edit history", () => {
  function withHistory(...scripts: Script[]) {
    const records: (AgentTurn | undefined)[] = [];
    const harness = setupWith(
      {
        checkResolve: async () => ({ state: "connected", product: "DaVinci Resolve Studio", version: "21", project: "Doc", timeline: null }),
        history: {
          record: async (turn) => {
            records.push(turn);
            return turn ? { commit: "abc1234", changes: ['"AI - Cut": new timeline, 3 clips, 0:10'] } : null;
          },
          recentContext: async () => "9f8e7d6 2026-11-07 10:00 Claude: Earlier cut",
          log: async () => [{ ...EARLIER_CUT, commit: `c${records.length}` }],
          timeline: async () => null,
          mediaFile: () => null,
        },
      },
      ...scripts,
    );
    return { ...harness, records };
  }

  it("saves before every turn and after a turn that changed Resolve, with what the agent did", async () => {
    const { app, client, send, turnDone, records, prompts } = withHistory(
      async (sink) => {
        sink.upsert({ id: "w", kind: "tool", name: "mcp__resolve__timeline", input: { action: "create_empty", params: { name: "AI - Cut" } }, status: "running" });
        sink.upsert({ id: "r", kind: "tool", name: "mcp__resolve__timeline", input: { action: "list" }, status: "done" });
        sink.upsert({ id: "w", status: "done" });
        sink.upsert({ id: "a", kind: "assistant", text: "Built AI - Cut." });
      },
      async (sink) => {
        sink.upsert({ id: "r2", kind: "tool", name: "mcp__resolve__timeline", input: { action: "list" }, status: "done" });
      },
    );
    await app.refreshResolve();
    send({ type: "send", text: "cut it" });
    await turnDone();
    send({ type: "send", text: "what is on it?" });
    await turnDone();

    expect(records).toEqual([
      undefined,
      { provider: "claude", prompt: "cut it", actions: ['timeline.create_empty {"name":"AI - Cut"}'], summary: "Built AI - Cut." },
      undefined,
    ]);
    expect(prompts[0]).toMatch(/^<edit_history>\n.*\n9f8e7d6 2026-11-07 10:00 Claude: Earlier cut\n<\/edit_history>\n\ncut it$/);
    expect(prompts[1]).toBe("what is on it?");
    expect(client.state.items.filter((item) => item.kind === "notice")).toMatchObject([
      { tone: "info", text: 'Saved to edit history (abc1234): "AI - Cut": new timeline, 3 clips, 0:10' },
    ]);
  });

  it("sends the history on request and to every client after each save", async () => {
    const { app, client, send, turnDone } = withHistory(async (sink) => {
      sink.upsert({ id: "w", kind: "tool", name: "mcp__resolve__timeline", input: { action: "create_empty" }, status: "done" });
    });
    send({ type: "load_history" });
    await app.refreshResolve();
    send({ type: "send", text: "cut it" });
    await turnDone();
    expect(client.histories.map((entries) => entries.map((entry) => entry.commit))).toEqual([["c0"], ["c2"]]);
  });

  it("still saves a turn's Resolve changes after its chat was reset", async () => {
    const { open, opened } = gate();
    const { app, client, send, turnDone, records } = withHistory(async (sink) => {
      sink.upsert({ id: "w", kind: "tool", name: "mcp__resolve__timeline", input: { action: "create_empty" }, status: "running" });
      await opened;
    });
    await app.refreshResolve();
    send({ type: "send", text: "cut it" });
    await client.until((state) => state.items.some((item) => item.id === "w"));
    send({ type: "new_chat" });
    open();
    await turnDone();
    expect(records.at(-1)).toMatchObject({ prompt: "cut it", actions: ["timeline.create_empty"] });
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
        checkResolve: async () => ({ state: "connected", product: "DaVinci Resolve Studio", version: "21", project: "Doc", timeline: null }),
        history: {
          record: async (turn) => (turn ? { commit: "abc1234", changes: [] } : null),
          recentContext: async () => "",
          log: async () => [],
          timeline: async () => null,
          mediaFile: () => null,
        },
      },
      async (sink) => {
        sink.upsert({ id: "w", kind: "tool", name: "mcp__resolve__timeline", input: { action: "create_empty" }, status: "done" });
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
