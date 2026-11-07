import { randomUUID } from "node:crypto";
import type {
  AppMeta,
  ApprovalMode,
  ApprovalRequest,
  ChatItem,
  ClientMessage,
  Provider,
  ResolveStatus,
  ServerMessage,
} from "../shared/protocol.ts";
import { resolveWriteAction, withHistoryContext, type AgentTurn, type History } from "./history/history.ts";
import type { ItemPatch, ProviderSession, TurnSink } from "./providers/types.ts";

export interface AppOptions {
  workspaceDir: string;
  createSession: (provider: Provider, approvalMode: () => ApprovalMode) => ProviderSession;
  checkResolve: () => Promise<ResolveStatus>;
  /** Saves Resolve's state to the workspace's Git history around turns that change it. */
  history?: History;
}

export interface Client {
  send(data: string): void;
}

export type App = ReturnType<typeof createApp>;

/** All app state lives here; clients get a snapshot on connect and patches after. */
export function createApp(options: AppOptions) {
  let provider: Provider = "claude";
  let approvalMode: ApprovalMode = "ask";
  let resolve: ResolveStatus = { state: "checking" };
  let items: ChatItem[] = [];
  let turn: AbortController | null = null;
  // Bumped on every reset, so a turn that is still winding down can't write into the next chat.
  let chat = 0;
  const approvals = new Map<string, { request: ApprovalRequest; settle: (allow: boolean) => void }>();
  let session = newSession();
  const clients = new Set<Client>();

  function newSession(): ProviderSession {
    return options.createSession(provider, () => approvalMode);
  }

  function broadcast(message: ServerMessage) {
    const data = JSON.stringify(message);
    for (const client of clients) client.send(data);
  }

  function meta(): AppMeta {
    return {
      provider,
      approvalMode,
      busy: turn !== null,
      approvals: [...approvals.values()].map((entry) => entry.request),
      resolve,
      workspaceDir: options.workspaceDir,
    };
  }

  const sendMeta = () => broadcast({ type: "meta", meta: meta() });
  const snapshot = (): ServerMessage => ({ type: "snapshot", state: { ...meta(), items } });

  function upsert(patch: ItemPatch) {
    const index = items.findIndex((item) => item.id === patch.id);
    const merged = { ...(index === -1 ? {} : items[index]), ...patch } as ChatItem;
    if (index === -1) items.push(merged);
    else items[index] = merged;
    broadcast({ type: "item", item: merged });
  }

  function settleAllApprovals(allow: boolean) {
    for (const entry of approvals.values()) entry.settle(allow);
    approvals.clear();
  }

  async function runTurn(text: string) {
    if (turn) return;
    const controller = new AbortController();
    turn = controller;
    const turnChat = chat;
    const turnProvider = provider;
    const live = () => turnChat === chat;
    const firstInChat = !items.some((item) => item.kind === "user");
    upsert({ id: randomUUID(), kind: "user", text });
    sendMeta();

    // Kept outside the chat items, so a turn's Resolve writes are still saved if its chat is reset.
    const tools = new Map<string, { name?: string; input?: unknown }>();
    let reply = "";

    const sink: TurnSink = {
      upsert: (patch) => {
        if (patch.kind === "tool" || tools.has(patch.id)) {
          const tool = tools.get(patch.id) ?? {};
          tools.set(patch.id, { name: patch.name ?? tool.name, input: patch.input ?? tool.input });
        }
        if (patch.kind === "assistant" && patch.text) reply = patch.text;
        if (live()) upsert(patch);
      },
      notice: (noticeText, tone = "info") => {
        if (live()) upsert({ id: randomUUID(), kind: "notice", text: noticeText, tone });
      },
      requestApproval: (tool, input) => {
        if (!live() || controller.signal.aborted) return Promise.resolve(false);
        return new Promise<boolean>((settle) => {
          const id = randomUUID();
          approvals.set(id, { request: { id, tool, input }, settle });
          sendMeta();
        });
      },
    };

    async function saveHistory(agentTurn?: AgentTurn) {
      try {
        const saved = await options.history!.record(agentTurn);
        if (saved) {
          await publishHistory();
          const who = agentTurn ? "Saved to edit history" : "Saved your own Resolve changes to edit history";
          sink.notice(`${who} (${saved.commit})${saved.changes.length ? `: ${saved.changes.join("; ")}` : "."}`);
        }
      } catch (error) {
        sink.notice(`Couldn't save edit history: ${error instanceof Error ? error.message : String(error)}`, "error");
      }
    }

    try {
      let prompt = text;
      if (options.history) {
        if (resolve.state === "connected") await saveHistory();
        if (firstInChat) prompt = withHistoryContext(await options.history.recentContext().catch(() => ""), text);
      }
      if (!controller.signal.aborted) await session.send(prompt, sink, controller.signal);
    } catch (error) {
      if (!controller.signal.aborted) {
        sink.notice(error instanceof Error ? error.message : String(error), "error");
      }
    } finally {
      if (controller.signal.aborted) sink.notice("Stopped.");
      for (const item of items) {
        if (item.kind === "tool" && item.status === "running") sink.upsert({ id: item.id, status: "error" });
      }
      settleAllApprovals(false);
      const actions = [...tools.values()].flatMap(({ name, input }) => resolveWriteAction(name, input) ?? []);
      if (options.history && actions.length) {
        await saveHistory({ provider: turnProvider, prompt: text, actions, summary: reply });
      }
      turn = null;
      sendMeta();
      void refreshResolve();
    }
  }

  function stopTurn() {
    turn?.abort();
    settleAllApprovals(false);
  }

  function resetChat() {
    stopTurn();
    chat += 1;
    items = [];
    session = newSession();
    broadcast(snapshot());
  }

  // History is shared by everyone looking at this workspace, so every client gets each update.
  async function publishHistory() {
    const entries = (await options.history?.log().catch(() => [])) ?? [];
    broadcast({ type: "history", entries });
  }

  let resolveCheck: Promise<void> | null = null;
  function refreshResolve(): Promise<void> {
    resolveCheck ??= options
      .checkResolve()
      .then((status) => {
        resolve = status;
        sendMeta();
      })
      .finally(() => {
        resolveCheck = null;
      });
    return resolveCheck;
  }

  function handleMessage(message: ClientMessage) {
    switch (message.type) {
      case "send":
        if (message.text.trim()) void runTurn(message.text.trim());
        break;
      case "stop":
        stopTurn();
        sendMeta();
        break;
      case "new_chat":
        resetChat();
        break;
      case "set_provider":
        if (message.provider !== provider) {
          provider = message.provider;
          resetChat();
        }
        break;
      case "set_approval_mode":
        approvalMode = message.mode;
        if (approvalMode === "auto") settleAllApprovals(true);
        sendMeta();
        break;
      case "approve": {
        const entry = approvals.get(message.id);
        if (entry) {
          approvals.delete(message.id);
          entry.settle(message.allow);
          sendMeta();
        }
        break;
      }
      case "refresh_resolve":
        void refreshResolve();
        break;
      case "load_history":
        void publishHistory();
        break;
    }
  }

  return {
    connect(client: Client): () => void {
      clients.add(client);
      client.send(JSON.stringify(snapshot()));
      return () => clients.delete(client);
    },
    handleMessage,
    refreshResolve,
    mediaFile: (id: string) => options.history?.mediaFile(id) ?? null,
    busy: () => turn !== null,
  };
}
