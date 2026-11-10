import { randomUUID } from "node:crypto";
import type { ApprovalMode, ApprovalRequest, ChatItem, ContextUsage, Provider, ThreadSummary } from "../shared/protocol.ts";
import { handoffSummary, withHandoff } from "./handoff.ts";
import { resolveWriteAction, withHistoryContext, type AgentTurn, type History } from "./history/history.ts";
import { threadTitle, type SavedThread } from "./projects/store.ts";
import type { ItemPatch, ProviderSession, TurnSink } from "./providers/types.ts";

/** What a thread needs from the app around it. */
export interface ThreadHost {
  approvalMode(): ApprovalMode;
  createSession(provider: Provider, resumeId?: string): ProviderSession;
  /** The project's edit history; undefined when saving is off. */
  history(): History | undefined;
  /** Takes Resolve for a turn; false while another turn, a render or an open has it. */
  claim(): boolean;
  release(): void;
  /** Rechecks Resolve before a turn: why this thread can't run against it now, or a notice when it can. */
  check(): Promise<{ refusal: string } | { notice: string | null }>;
  resolveConnected(): boolean;
  /** Other people may change the project too, as in a Blackmagic Cloud collaboration. */
  shared(): boolean;
  /** A save landed in the project's history. */
  saved(): Promise<void>;
  /** Anything in the thread's meta or summary changed. */
  changed(): void;
  item(item: ChatItem): void;
  /** The message didn't go to the agent; the client puts it back. */
  refused(text: string, reason: string): void;
  persist(thread: SavedThread): void;
  refreshResolve(): void;
}

const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude", codex: "Codex" };

export const DEFAULT_TITLE = "New thread";

export type Thread = ReturnType<typeof createThread>;

/** One conversation with an agent: its chat, its provider session and its turn. */
export function createThread(saved: SavedThread, host: ThreadHost) {
  const id = saved.id;
  let title = saved.title;
  let provider: Provider = saved.provider;
  let items: ChatItem[] = saved.items;
  let context: ContextUsage | null = saved.context;
  const createdAt = saved.createdAt;
  let updatedAt = saved.updatedAt;
  let archivedAt = saved.archivedAt;
  let turn: AbortController | null = null;
  // The running turn's sink, for approvals asked outside the provider session, as by the Resolve guard.
  let activeSink: TurnSink | null = null;
  // Set once the thread is deleted, so a turn that is still winding down can't write into it.
  let disposed = false;
  const approvals = new Map<string, { request: ApprovalRequest; settle: (allow: boolean) => void }>();
  let session = host.createSession(provider, saved.resumeId ?? undefined);
  // Until a turn of it succeeds: the provider may have dropped the conversation since.
  let resumed = saved.resumeId != null;

  function record(): SavedThread {
    return { id, title, provider, resumeId: session.resumeId(), items, context, createdAt, updatedAt, archivedAt };
  }

  // An empty thread is kept in memory only, so unused new threads don't pile up across restarts.
  function save() {
    if (!disposed && items.length > 0) host.persist(record());
  }

  function upsert(patch: ItemPatch) {
    const index = items.findIndex((item) => item.id === patch.id);
    const merged = { ...(index === -1 ? {} : items[index]), ...patch } as ChatItem;
    if (index === -1) items.push(merged);
    else items[index] = merged;
    if (merged.kind === "user" && title === DEFAULT_TITLE) title = threadTitle(items);
    host.item(merged);
  }

  function settleAllApprovals(allow: boolean) {
    for (const entry of approvals.values()) entry.settle(allow);
    approvals.clear();
  }

  /** A new session knows nothing yet: it gets the edit history, and a summary of the chat when one is going. */
  async function primeNewSession(text: string, earlier: ChatItem[]): Promise<string> {
    const prompt = withHandoff(handoffSummary(earlier), text);
    const history = host.history();
    if (!history) return prompt;
    return withHistoryContext(await history.recentContext().catch(() => ""), prompt);
  }

  /** A null text only compacts the chat. */
  async function runTurn(text: string | null, compactFirst = false) {
    if (turn || disposed) return;
    if (text === null && !(session.compact && session.resumeId())) return;
    // Resolve does one thing at a time: a render switches timelines and pages while it runs.
    if (!host.claim()) {
      if (text !== null) host.refused(text, "Resolve is busy with another thread's turn or a render. Send again when it's done.");
      return;
    }
    const controller = new AbortController();
    turn = controller;
    host.changed();

    const checked = await host.check().catch((error: unknown) => ({ refusal: error instanceof Error ? error.message : String(error) }));
    if ("refusal" in checked) {
      turn = null;
      host.release();
      host.changed();
      if (text !== null) host.refused(text, checked.refusal);
      return;
    }

    const turnProvider = provider;
    const turnSession = session;
    const live = () => !disposed;
    const earlier = [...items];
    updatedAt = new Date().toISOString();
    if (text !== null) upsert({ id: randomUUID(), kind: "user", text });
    if (checked.notice) upsert({ id: randomUUID(), kind: "notice", text: checked.notice, tone: "info" });
    host.changed();
    let produced = false;

    // Kept outside the chat items, so a turn's Resolve writes are still saved if its thread is deleted.
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
          const requestId = randomUUID();
          approvals.set(requestId, { request: { id: requestId, tool, input }, settle });
          host.changed();
        });
      },
      context: (usage) => {
        if (live() && session === turnSession) {
          context = usage;
          host.changed();
        }
      },
    };

    activeSink = sink;

    async function saveHistory(agentTurn?: AgentTurn) {
      try {
        const saved = await host.history()!.record(agentTurn);
        if (saved) {
          await host.saved();
          const who = agentTurn
            ? "Saved to edit history"
            : host.shared()
              ? "Saved changes made in Resolve outside Linker-Edit to edit history"
              : "Saved your own Resolve changes to edit history";
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
        if (host.history() && host.resolveConnected()) await saveHistory();
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
          session = host.createSession(provider);
          resumed = false;
          sink.notice(`Couldn't continue the earlier conversation (${message(error)}), so a new one started with a summary of this chat.`);
          await session.send(await primeNewSession(text, earlier), sink, signal);
        }
        if (session === turnSession) resumed = false;
      }
    } catch (error) {
      if (!signal.aborted) sink.notice(message(error), "error");
    } finally {
      activeSink = null;
      if (controller.signal.aborted) sink.notice("Stopped.");
      for (const item of items) {
        if (item.kind === "tool" && item.status === "running") sink.upsert({ id: item.id, status: "error" });
      }
      settleAllApprovals(false);
      const actions = [...tools.values()].flatMap(({ name, input }) => resolveWriteAction(name, input) ?? []);
      if (host.history() && text !== null && actions.length) {
        await saveHistory({ provider: turnProvider, prompt: text, actions, summary: reply, thread: id });
      }
      turn = null;
      updatedAt = new Date().toISOString();
      host.release();
      host.changed();
      save();
      host.refreshResolve();
    }
  }

  function stop() {
    turn?.abort();
    settleAllApprovals(false);
  }

  /** The chat goes on; the new agent gets a summary of it with the next message. */
  function setProvider(next: Provider) {
    if (next === provider) return;
    stop();
    provider = next;
    session = host.createSession(provider);
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
    host.changed();
    save();
  }

  return {
    id,
    get items() {
      return items;
    },
    get provider() {
      return provider;
    },
    get context() {
      return context;
    },
    get busy() {
      return turn !== null;
    },
    get archived() {
      return archivedAt !== null;
    },
    get updatedAt() {
      return updatedAt;
    },
    get title() {
      return title;
    },
    get empty() {
      return items.length === 0;
    },
    approvals: () => [...approvals.values()].map((entry) => entry.request),
    summary: (): ThreadSummary => ({
      id,
      title,
      updatedAt,
      archived: archivedAt !== null,
      busy: turn !== null,
      waiting: approvals.size > 0,
    }),
    send: (text: string, compactFirst: boolean) => void runTurn(text, compactFirst),
    compact: () => void runTurn(null),
    stop() {
      stop();
      host.changed();
    },
    setProvider,
    /** True when the request was this thread's. */
    approve(requestId: string, allow: boolean): boolean {
      const entry = approvals.get(requestId);
      if (!entry) return false;
      approvals.delete(requestId);
      entry.settle(allow);
      host.changed();
      return true;
    },
    settleAllApprovals,
    /** Asks the user during this thread's turn; false when no turn is running. */
    requestApproval: (tool: string, input: unknown): Promise<boolean> =>
      activeSink ? activeSink.requestApproval(tool, input) : Promise.resolve(false),
    rename(next: string) {
      title = next.trim().slice(0, 120) || title;
      host.changed();
      save();
    },
    setArchived(archived: boolean) {
      if (archived) stop();
      archivedAt = archived ? new Date().toISOString() : null;
      host.changed();
      save();
    },
    /** Stops the turn; a turn still winding down saves its Resolve changes but writes nothing here. */
    dispose() {
      stop();
      disposed = true;
    },
  };
}
