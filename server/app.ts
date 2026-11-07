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
import type { ItemPatch, ProviderSession, TurnSink } from "./providers/types.ts";

export interface AppOptions {
  workspaceDir: string;
  createSession: (provider: Provider, approvalMode: () => ApprovalMode) => ProviderSession;
  checkResolve: () => Promise<ResolveStatus>;
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
    const live = () => turnChat === chat;
    upsert({ id: randomUUID(), kind: "user", text });
    sendMeta();

    const sink: TurnSink = {
      upsert: (patch) => {
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

    try {
      await session.send(text, sink, controller.signal);
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
    busy: () => turn !== null,
  };
}
