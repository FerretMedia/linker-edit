import { randomUUID } from "node:crypto";
import * as path from "node:path";
import type {
  AppMeta,
  ApprovalMode,
  ApprovalRequest,
  ChatItem,
  ClientMessage,
  ContextUsage,
  PreviewState,
  Provider,
  ResolveStatus,
  ServerMessage,
} from "../shared/protocol.ts";
import type { ChatStore } from "./chatStore.ts";
import { handoffSummary, withHandoff } from "./handoff.ts";
import { resolveWriteAction, withHistoryContext, type AgentTurn, type History } from "./history/history.ts";
import type { PreviewRenderer } from "./preview/preview.ts";
import type { ItemPatch, ProviderSession, TurnSink } from "./providers/types.ts";

export interface AppOptions {
  version: string;
  workspaceDir: string;
  /** resumeId continues a conversation from before a restart. */
  createSession: (provider: Provider, approvalMode: () => ApprovalMode, resumeId?: string) => ProviderSession;
  checkResolve: () => Promise<ResolveStatus>;
  /** Saves Resolve's state to the workspace's Git history around turns that change it. */
  history?: History;
  /** Renders timelines through Resolve for the preview panel. */
  preview?: PreviewRenderer;
  /** Keeps the open chat across restarts. */
  store?: ChatStore;
}

const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude", codex: "Codex" };

export interface Client {
  send(data: string): void;
}

export type App = ReturnType<typeof createApp>;

/** All app state lives here; clients get a snapshot on connect and patches after. */
export function createApp(options: AppOptions) {
  const saved = options.store?.load() ?? null;
  let provider: Provider = saved?.provider ?? "claude";
  let approvalMode: ApprovalMode = "ask";
  let resolve: ResolveStatus = { state: "checking" };
  let items: ChatItem[] = saved?.items ?? [];
  let context: ContextUsage | null = saved?.context ?? null;
  let turn: AbortController | null = null;
  let preview: PreviewState = { state: "idle" };
  let previewJob: { stop(): void } | null = null;
  let update: string | null = null;
  // Bumped on every reset, so a turn that is still winding down can't write into the next chat.
  let chat = 0;
  const approvals = new Map<string, { request: ApprovalRequest; settle: (allow: boolean) => void }>();
  let session = newSession(saved?.resumeId ?? undefined);
  // Until a turn of it succeeds: the provider may have dropped the conversation since.
  let resumed = saved?.resumeId != null;
  const clients = new Set<Client>();

  function newSession(resumeId?: string): ProviderSession {
    return options.createSession(provider, () => approvalMode, resumeId);
  }

  function save() {
    options.store?.save({ provider, resumeId: session.resumeId(), items, context });
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
      version: options.version,
      update,
      workspaceDir: options.workspaceDir,
      preview,
      context,
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

  /** A new session knows nothing yet: it gets the edit history, and a summary of the chat when one is going. */
  async function primeNewSession(text: string, earlier: ChatItem[]): Promise<string> {
    const prompt = withHandoff(handoffSummary(earlier), text);
    if (!options.history) return prompt;
    return withHistoryContext(await options.history.recentContext().catch(() => ""), prompt);
  }

  /** A null text only compacts the chat. */
  async function runTurn(text: string | null, compactFirst = false) {
    // Resolve does one thing at a time: a render switches timelines and pages while it runs.
    if (turn || previewJob) return;
    if (text === null && !(session.compact && session.resumeId())) return;
    const controller = new AbortController();
    turn = controller;
    const turnChat = chat;
    const turnProvider = provider;
    const turnSession = session;
    const live = () => turnChat === chat;
    const earlier = [...items];
    if (text !== null) upsert({ id: randomUUID(), kind: "user", text });
    sendMeta();
    let produced = false;

    // Kept outside the chat items, so a turn's Resolve writes are still saved if its chat is reset.
    const tools = new Map<string, { name?: string; input?: unknown }>();
    let reply = "";

    const sink: TurnSink = {
      upsert: (patch) => {
        produced = true;
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
      context: (usage) => {
        if (live() && session === turnSession) {
          context = usage;
          sendMeta();
        }
      },
    };

    async function saveHistory(agentTurn?: AgentTurn) {
      try {
        const saved = await options.history!.record(agentTurn);
        if (saved) {
          if (preview.state === "ready") preview = { ...preview, outdated: true };
          await publishHistory();
          const who = agentTurn ? "Saved to edit history" : "Saved your own Resolve changes to edit history";
          sink.notice(`${who} (${saved.commit})${saved.changes.length ? `: ${saved.changes.join("; ")}` : "."}`);
        }
      } catch (error) {
        sink.notice(`Couldn't save edit history: ${error instanceof Error ? error.message : String(error)}`, "error");
      }
    }

    const { signal } = controller;
    const message = (error: unknown) => (error instanceof Error ? error.message : String(error));
    try {
      if (text === null) {
        await turnSession.compact!(sink, signal);
      } else {
        if (options.history && resolve.state === "connected") await saveHistory();
        if (compactFirst && turnSession.compact && turnSession.resumeId() && !signal.aborted) {
          // A failed compaction shouldn't cost the user their message.
          await turnSession.compact(sink, signal).catch((error: unknown) => {
            if (!signal.aborted) sink.notice(`Couldn't compact the chat: ${message(error)}`, "error");
          });
        }
        const prompt = turnSession.resumeId() ? text : await primeNewSession(text, earlier);
        try {
          if (!signal.aborted) await turnSession.send(prompt, sink, signal);
        } catch (error) {
          if (!resumed || produced || signal.aborted || !live() || session !== turnSession) throw error;
          session = newSession();
          resumed = false;
          sink.notice(`Couldn't continue the earlier conversation (${message(error)}), so a new one started with a summary of this chat.`);
          await session.send(await primeNewSession(text, earlier), sink, signal);
        }
        if (session === turnSession) resumed = false;
      }
    } catch (error) {
      if (!signal.aborted) sink.notice(message(error), "error");
    } finally {
      if (controller.signal.aborted) sink.notice("Stopped.");
      for (const item of items) {
        if (item.kind === "tool" && item.status === "running") sink.upsert({ id: item.id, status: "error" });
      }
      settleAllApprovals(false);
      const actions = [...tools.values()].flatMap(({ name, input }) => resolveWriteAction(name, input) ?? []);
      if (options.history && text !== null && actions.length) {
        await saveHistory({ provider: turnProvider, prompt: text, actions, summary: reply });
      }
      turn = null;
      sendMeta();
      save();
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
    context = null;
    session = newSession();
    resumed = false;
    broadcast(snapshot());
    save();
  }

  /** The chat goes on; the new agent gets a summary of it with the next message. */
  function switchProvider(next: Provider) {
    stopTurn();
    provider = next;
    session = newSession();
    resumed = false;
    context = null;
    if (items.some((item) => item.kind === "user")) {
      upsert({
        id: randomUUID(),
        kind: "notice",
        text: `Switched to ${PROVIDER_NAME[next]}. It gets a summary of this chat with your next message.`,
        tone: "info",
      });
    }
    sendMeta();
    save();
  }

  // History is shared by everyone looking at this workspace, so every client gets each update.
  async function publishHistory() {
    const entries = (await options.history?.log().catch(() => [])) ?? [];
    broadcast({ type: "history", entries });
  }

  function renderPreview(timelineId: string | null) {
    if (!options.preview || turn || previewJob) return;
    preview = { state: "rendering", timeline: null, percent: 0 };
    sendMeta();
    previewJob = options.preview.render(timelineId, (event) => {
      if (event.event === "start") preview = { state: "rendering", timeline: event.timeline, percent: 0 };
      else if (event.event === "progress" && preview.state === "rendering") preview = { ...preview, percent: event.percent };
      else if (event.event === "done" || event.event === "error") {
        const timeline = preview.state === "rendering" ? preview.timeline : null;
        previewJob = null;
        if (event.event === "error") preview = event.stopped ? { state: "idle" } : { state: "error", message: event.message };
        else if (!timeline) preview = { state: "error", message: "Resolve rendered without naming the timeline." };
        else {
          const url = `/preview/${encodeURIComponent(path.basename(event.file))}`;
          preview = { state: "ready", timeline, url, renderedAt: new Date().toISOString(), outdated: false };
        }
      }
      sendMeta();
    });
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
        if (message.text.trim()) void runTurn(message.text.trim(), message.compactFirst === true);
        break;
      case "compact":
        void runTurn(null);
        break;
      case "stop":
        stopTurn();
        sendMeta();
        break;
      case "new_chat":
        resetChat();
        break;
      case "set_provider":
        if (message.provider !== provider) switchProvider(message.provider);
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
      case "render_preview":
        renderPreview(message.timelineId ?? null);
        break;
      case "stop_preview":
        previewJob?.stop();
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
    /** A newer release the UI offers, or null. */
    setUpdate(version: string | null) {
      if (version === update) return;
      update = version;
      sendMeta();
    },
    mediaFile: (id: string) => options.history?.mediaFile(id) ?? null,
    previewFile: (name: string) => options.preview?.file(name) ?? null,
    playableTimeline: async (id: string | null, commit: string | null) => {
      if (!options.history) throw new Error("Edit history is off, so there is nothing to play.");
      return options.history.timeline(id, commit);
    },
    busy: () => turn !== null,
  };
}
