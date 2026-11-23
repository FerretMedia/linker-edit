// Stand-ins for the user's other computers: a page-side WebSocket that answers like a Linker-Edit server, with the
// e2e server's own state as the template, so the page gets a whole AppState.
import type { Page, WebSocketRoute } from "@playwright/test";
import type { AppState, ClientMessage, ProjectSummary, ThreadSummary } from "../shared/protocol.ts";

const NOW = Date.now();
const ago = (minutes: number) => new Date(NOW - minutes * 60_000).toISOString();

/** The e2e server's first snapshot, as every fake machine's starting point. */
export async function templateState(baseURL: string): Promise<AppState> {
  const socket = new WebSocket(`${baseURL.replace(/^http/, "ws")}/ws?meta=patch`);
  try {
    return await new Promise<AppState>((resolve, reject) => {
      socket.onerror = () => reject(new Error("No e2e server"));
      socket.onmessage = (event) => {
        const message = JSON.parse(String(event.data)) as { type: string; state?: AppState };
        if (message.type === "snapshot" && message.state) resolve(message.state);
      };
    });
  } finally {
    socket.close();
  }
}

export function thread(id: string, title: string, fields: Partial<ThreadSummary> = {}): ThreadSummary {
  return {
    id,
    title,
    createdAt: ago(120),
    updatedAt: ago(5),
    archived: false,
    busy: false,
    waiting: false,
    pinnedAt: null,
    settledAt: null,
    unsettledAt: null,
    snoozedUntil: null,
    snoozedAt: null,
    lastReadAt: ago(10),
    lastSentAt: ago(5),
    turnEndedAt: null,
    ...fields,
  };
}

export function project(id: string, name: string, threads: ThreadSummary[]): ProjectSummary {
  return { id, name, dir: `/projects/${id}`, link: null, local: {}, open: false, threads, icon: null, pinned: false, archived: false, setup: false, autonomous: false };
}

/**
 * Serves a machine's WebSocket from the page's side: a snapshot of `computer` with `projects`, and a new one for
 * each thread the page selects. `at` is the socket's path, or a test of its whole address. Returns what the page
 * sent it.
 */
export async function fakeMachine(page: Page, at: string | ((url: URL) => boolean), template: AppState, computer: string, projects: ProjectSummary[]) {
  const sent: ClientMessage[] = [];
  const stateFor = (threadId: string | null): AppState => {
    const open = projects.find((entry) => entry.threads.some((candidate) => candidate.id === threadId));
    return { ...template, computer, projects, threadId, projectId: open?.id ?? null, items: [], busy: false, approvals: [], blocked: null };
  };
  await page.routeWebSocket(
    typeof at === "string" ? (url) => url.pathname === at : at,
    (socket: WebSocketRoute) => {
      socket.send(JSON.stringify({ type: "snapshot", state: stateFor(projects[0]?.threads[0]?.id ?? null) }));
      socket.onMessage((data) => {
        const message = JSON.parse(String(data)) as ClientMessage;
        sent.push(message);
        if (message.type === "select_thread") socket.send(JSON.stringify({ type: "snapshot", state: stateFor(message.threadId) }));
      });
    },
  );
  return sent;
}
