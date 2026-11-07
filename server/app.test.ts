import { describe, expect, it } from "vitest";
import { applyServerMessage } from "../src/lib/appState.ts";
import type { AppState, ClientMessage, Provider, ServerMessage } from "../shared/protocol.ts";
import { createApp } from "./app.ts";
import type { ProviderSession, TurnSink } from "./providers/types.ts";

type Script = (sink: TurnSink, signal: AbortSignal) => Promise<void>;

/** A client that keeps state the way the UI does, and resolves waits as messages arrive. */
function testClient() {
  let state: AppState | null = null;
  const waiters: { check: () => boolean; resolve: () => void }[] = [];
  return {
    send(data: string) {
      state = applyServerMessage(state, JSON.parse(data) as ServerMessage);
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
  const sessions: Provider[] = [];
  const app = createApp({
    workspaceDir: "/work",
    createSession: (provider): ProviderSession => {
      sessions.push(provider);
      return {
        send: (_text, sink, signal) => {
          const script = scripts.shift();
          if (!script) throw new Error("No script left for this send");
          return script(sink, signal);
        },
      };
    },
    checkResolve: async () => ({ state: "not_running" }),
  });
  const client = testClient();
  app.connect(client);
  const send = (message: ClientMessage) => app.handleMessage(message);
  const turnDone = () => client.until((state) => !state.busy);
  return { app, client, send, turnDone, sessions };
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
