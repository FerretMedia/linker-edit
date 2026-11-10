import { randomUUID } from "node:crypto";
import * as path from "node:path";
import type {
  AppMeta,
  ApprovalMode,
  ChatItem,
  ClientMessage,
  CloudSyncMode,
  LocalLink,
  PreviewState,
  ProjectSource,
  ProjectSummary,
  Provider,
  ResolveLink,
  ResolveStatus,
  ServerMessage,
} from "../shared/protocol.ts";
import type { History, ProjectCheck } from "./history/history.ts";
import { isLinkedProject, linkFromOpen, linkKind, openProject, refreshedLink, type OpenProject } from "./projects/link.ts";
import type { ProjectStore, SavedProject, SavedThread } from "./projects/store.ts";
import type { PreviewRenderer } from "./preview/preview.ts";
import type { ProviderSession } from "./providers/types.ts";
import type { RemoteAccessService } from "./remote.ts";
import type { ResolveProjects } from "./resolveProjects.ts";
import { createThread, DEFAULT_TITLE, type Thread } from "./thread.ts";
import { describeResults, type Transcriber } from "./transcribe/transcriber.ts";

export interface AppOptions {
  version: string;
  store: ProjectStore;
  /** resumeId continues a conversation from before a restart; dir is the project's working folder. */
  createSession: (provider: Provider, approvalMode: () => ApprovalMode, dir: string, resumeId?: string) => ProviderSession;
  checkResolve: () => Promise<ResolveStatus>;
  /** Each project's edit history in its folder; saving is off without it. */
  createHistory?: (dir: string, checkProject: ProjectCheck) => History;
  /** Renders timelines through Resolve for the preview panel. */
  preview?: PreviewRenderer;
  /** Lists and opens Resolve projects; without it projects can only follow what Resolve has open. */
  resolveProjects?: ResolveProjects;
  /** Gets a project folder ready for agents, such as its AGENTS.md. */
  prepareFolder?: (dir: string) => void;
  /** Speech to text for the agents' transcribe tool. */
  transcriber?: Transcriber;
  /** Opening the app from the user's other devices over Tailscale. */
  remote?: RemoteAccessService;
}

const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude", codex: "Codex" };

export interface Client {
  send(data: string): void;
}

export type App = ReturnType<typeof createApp>;

interface ProjectState {
  saved: SavedProject;
  history?: History;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** All app state lives here; clients get a snapshot of the thread they look at, and patches after. */
export function createApp(options: AppOptions) {
  const { store } = options;
  let approvalMode: ApprovalMode = "ask";
  let resolve: ResolveStatus = { state: "checking" };
  let preview: PreviewState = { state: "idle" };
  let previewJob: { stop(): void } | null = null;
  let update: string | null = null;
  // Resolve has one project open and does one thing at a time: one turn, open or render app-wide.
  let turnOwner: string | null = null;
  let opening: string | null = null;
  // New threads start with the agent picked last.
  let lastProvider: Provider = "claude";
  const projects = new Map<string, ProjectState>();
  const threads = new Map<string, { thread: Thread; projectId: string }>();
  const clients = new Map<Client, { threadId: string | null }>();

  const projectName = (saved: SavedProject) => saved.name ?? saved.link?.name ?? path.basename(saved.dir);
  const resolveBusy = () => turnOwner !== null || previewJob !== null || opening !== null;

  function addProject(saved: SavedProject, savedThreads: SavedThread[] = []): ProjectState {
    try {
      options.prepareFolder?.(saved.dir);
    } catch (error) {
      console.error(error);
    }
    const state: ProjectState = { saved };
    state.history = options.createHistory?.(saved.dir, (open) => {
      const link = state.saved.link;
      if (!link) return null;
      const same = link.projectId && open.id ? link.projectId === open.id : link.name === open.name;
      return same ? null : `Resolve has "${open.name}" open, not this project's "${link.name}", so its timelines weren't saved here.`;
    });
    projects.set(saved.id, state);
    for (const thread of savedThreads) addThread(saved.id, thread);
    return state;
  }

  function addThread(projectId: string, saved: SavedThread): Thread {
    // Held, not looked up: a turn still winding down after its project was removed saves into its folder.
    const project = projects.get(projectId)!;
    const thread = createThread(saved, {
      approvalMode: () => approvalMode,
      createSession: (provider, resumeId) => options.createSession(provider, () => approvalMode, project.saved.dir, resumeId),
      history: () => project.history,
      claim() {
        if (resolveBusy()) return false;
        turnOwner = saved.id;
        return true;
      },
      release() {
        if (turnOwner === saved.id) turnOwner = null;
      },
      check: () => checkBeforeTurn(project),
      resolveConnected: () => resolve.state === "connected",
      shared: () => project.saved.link?.kind === "cloud",
      saved: async () => {
        if (preview.state === "ready") preview = { ...preview, outdated: true };
        await publishHistory(projectId);
      },
      changed: sendMeta,
      item: (item) => sendItem(saved.id, item),
      refused(text, reason) {
        for (const [client, view] of clients) {
          if (view.threadId === saved.id) {
            send(client, { type: "refused", threadId: saved.id, text });
            send(client, { type: "toast", text: reason, tone: "error" });
          }
        }
      },
      persist: (record) => store.saveThread(project.saved, record),
      refreshResolve: () => void refreshResolve(),
    });
    threads.set(saved.id, { thread, projectId });
    return thread;
  }

  function newThread(projectId: string): Thread {
    // An untouched new thread is reused rather than piling up.
    const empty = projectThreads(projectId).find((thread) => thread.empty && !thread.archived && !thread.busy);
    if (empty) return empty;
    const now = new Date().toISOString();
    return addThread(projectId, {
      id: randomUUID(),
      title: DEFAULT_TITLE,
      provider: lastProvider,
      resumeId: null,
      items: [],
      context: null,
      createdAt: now,
      updatedAt: now,
      archivedAt: null,
    });
  }

  for (const { project, threads: saved } of store.load()) addProject(project, saved);

  function projectThreads(projectId: string): Thread[] {
    return [...threads.values()]
      .filter((entry) => entry.projectId === projectId)
      .map((entry) => entry.thread)
      .sort((a, b) => b.updatedAt.localeCompare(a.updatedAt));
  }

  /** The project whose Resolve project is open, if Linker-Edit has one. */
  function ownerOf(open: OpenProject): ProjectState | undefined {
    return [...projects.values()].find((project) => project.saved.link && isLinkedProject(project.saved.link, project.saved.local, open));
  }

  /** Why a thread of this project can't run against Resolve now; null when it can. */
  function projectRefusal(project: ProjectState): string | null {
    const open = openProject(resolve);
    if (!open) return null;
    const { link } = project.saved;
    if (link) {
      if (isLinkedProject(link, project.saved.local, open)) return null;
      return `Resolve has "${open.name}" open, not this project's "${link.name}". Open "${link.name}" in Resolve to continue.`;
    }
    const owner = ownerOf(open);
    if (owner) {
      return `Resolve has "${open.name}" open, which belongs to the project "${projectName(owner.saved)}". Switch to that project, or open another one in Resolve.`;
    }
    return null;
  }

  async function checkBeforeTurn(project: ProjectState): Promise<{ refusal: string } | { notice: string | null }> {
    await refreshResolve();
    const refusal = projectRefusal(project);
    if (refusal) return { refusal };
    const open = openProject(resolve);
    if (!open || project.saved.link) return { notice: null };
    setLink(project, linkFromOpen(open), `Link to Resolve project "${open.name}"`);
    return { notice: `Linked this project to the Resolve project "${open.name}" in ${describeLibrary(project.saved.link!, open)}.` };
  }

  function describeLibrary(link: ResolveLink, open: OpenProject): string {
    if (link.kind === "cloud") return "Blackmagic Cloud";
    const name = open.database?.name;
    return link.kind === "network" ? `the network database${name ? ` "${name}"` : ""}` : `the local database${name ? ` "${name}"` : ""}`;
  }

  function setLink(project: ProjectState, next: { link: ResolveLink | null; local: LocalLink }, commitMessage: string | null) {
    const linkChanged = JSON.stringify(next.link) !== JSON.stringify(project.saved.link);
    project.saved = { ...project.saved, link: next.link, local: next.local };
    store.saveProject(project.saved);
    if (linkChanged && commitMessage) {
      project.history?.commitProjectFile(commitMessage).catch((error: unknown) => console.error("Couldn't commit linker-edit.json", error));
    }
    sendMeta();
  }

  function summaries(): ProjectSummary[] {
    const open = openProject(resolve);
    return [...projects.values()].map(({ saved }) => ({
      id: saved.id,
      name: projectName(saved),
      dir: saved.dir,
      link: saved.link,
      local: saved.local,
      open: !!(open && saved.link && isLinkedProject(saved.link, saved.local, open)),
      threads: projectThreads(saved.id).map((thread) => thread.summary()),
    }));
  }

  function blockedFor(thread: Thread, projectId: string): AppMeta["blocked"] {
    if (thread.busy) return null;
    if (opening) {
      const target = projects.get(opening);
      return { message: `Opening "${target ? projectName(target.saved) : "the project"}" in Resolve…`, canOpen: false };
    }
    const owner = turnOwner ? threads.get(turnOwner) : undefined;
    if (owner) {
      const where = owner.projectId === projectId ? "" : ` (${projectName(projects.get(owner.projectId)!.saved)})`;
      return {
        message: `${PROVIDER_NAME[owner.thread.provider]} is working in "${owner.thread.title}"${where}. One thread works in Resolve at a time.`,
        canOpen: false,
      };
    }
    const project = projects.get(projectId)!;
    const refusal = projectRefusal(project);
    return refusal ? { message: refusal, canOpen: !!project.saved.link && !!options.resolveProjects } : null;
  }

  function metaFor(threadId: string | null): AppMeta {
    const entry = threadId ? threads.get(threadId) : undefined;
    const project = entry ? projects.get(entry.projectId) : undefined;
    return {
      projects: summaries(),
      threadId: entry ? threadId : null,
      projectId: entry?.projectId ?? null,
      provider: entry?.thread.provider ?? lastProvider,
      approvalMode,
      busy: entry?.thread.busy ?? false,
      approvals: entry?.thread.approvals() ?? [],
      resolve,
      version: options.version,
      update,
      workspaceDir: project?.saved.dir ?? null,
      preview,
      context: entry?.thread.context ?? null,
      blocked: entry ? blockedFor(entry.thread, entry.projectId) : null,
      opening,
      transcription: options.transcriber?.status() ?? { state: "idle" },
      remote: options.remote?.status() ?? { state: "unavailable", hint: "Remote access isn't available in this build." },
    };
  }

  function send(client: Client, message: ServerMessage) {
    client.send(JSON.stringify(message));
  }

  function sendMeta() {
    for (const [client, view] of clients) send(client, { type: "meta", meta: metaFor(view.threadId) });
  }
  options.transcriber?.onStatus(sendMeta);
  options.remote?.onStatus(sendMeta);

  function sendSnapshot(client: Client) {
    const view = clients.get(client);
    if (!view) return;
    const items = view.threadId ? (threads.get(view.threadId)?.thread.items ?? []) : [];
    send(client, { type: "snapshot", state: { ...metaFor(view.threadId), items } });
  }

  function sendItem(threadId: string, item: ChatItem) {
    for (const [client, view] of clients) {
      if (view.threadId === threadId) send(client, { type: "item", threadId, item });
    }
  }

  function toast(client: Client | null, text: string, tone: "info" | "error" = "error") {
    for (const target of client ? [client] : clients.keys()) send(target, { type: "toast", text, tone });
  }

  /** Where a client lands: the project open in Resolve first, then its latest thread, or a new one. */
  function defaultThread(preferProject?: string): string | null {
    const open = openProject(resolve);
    const owner = open ? ownerOf(open) : undefined;
    const projectId = (preferProject && projects.has(preferProject) ? preferProject : undefined) ?? owner?.saved.id ?? projects.keys().next().value;
    if (!projectId) return null;
    return projectThreads(projectId).find((thread) => !thread.archived)?.id ?? newThread(projectId).id;
  }

  function select(client: Client, threadId: string | null) {
    clients.set(client, { threadId });
    sendSnapshot(client);
    sendMeta();
  }

  /** Clients looking at a thread that went away move to another thread of the same project. */
  function moveViewers(threadId: string, projectId: string) {
    for (const [client, view] of clients) {
      if (view.threadId === threadId) {
        clients.set(client, { threadId: defaultThread(projectId) });
        sendSnapshot(client);
      }
    }
    sendMeta();
  }

  function removeThread(threadId: string) {
    const entry = threads.get(threadId);
    if (!entry) return;
    entry.thread.dispose();
    threads.delete(threadId);
    store.deleteThread(projects.get(entry.projectId)!.saved, threadId);
    moveViewers(threadId, entry.projectId);
  }

  async function publishHistory(projectId: string, client?: Client) {
    const history = projects.get(projectId)?.history;
    if (!history) return;
    const entries = await history.log().catch(() => []);
    for (const target of client ? [client] : clients.keys()) send(target, { type: "history", projectId, entries });
  }

  function renderPreview(timelineId: string | null) {
    if (!options.preview || resolveBusy()) return;
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
        // Learns the project's id once it is seen open, and follows renames made in Resolve.
        const open = openProject(status);
        const owner = open ? ownerOf(open) : undefined;
        if (open && owner?.saved.link) {
          const next = refreshedLink(owner.saved.link, owner.saved.local, open);
          if (JSON.stringify(next) !== JSON.stringify({ link: owner.saved.link, local: owner.saved.local })) {
            const renamed = next.link.name !== owner.saved.link.name;
            setLink(owner, next, renamed ? `Resolve project renamed to "${open.name}"` : `Link to Resolve project "${open.name}"`);
          }
        }
        sendMeta();
      })
      .finally(() => {
        resolveCheck = null;
      });
    return resolveCheck;
  }

  function sameLink(a: { link: ResolveLink; local: LocalLink }, project: SavedProject): boolean {
    const b = project.link;
    if (!b || a.link.kind !== b.kind || a.link.name !== b.name) return false;
    if (a.link.projectId && b.projectId) return a.link.projectId === b.projectId;
    const dbA = a.local.database;
    const dbB = project.local.database;
    return !dbA || !dbB || (dbA.name === dbB.name && (dbA.host ?? "") === (dbB.host ?? ""));
  }

  function linkFromSource(source: Exclude<ProjectSource, { kind: "open" | "folder" }>): { link: ResolveLink; local: LocalLink } {
    if (source.kind === "cloud") {
      return {
        link: { kind: "cloud", name: source.name.trim(), projectId: null },
        local: { mediaPath: source.mediaPath.trim(), syncMode: source.syncMode },
      };
    }
    return {
      link: { kind: linkKind(source.database), name: source.name, projectId: null },
      local: { ...(source.database ? { database: source.database } : {}), folder: source.folder },
    };
  }

  function addProjectFrom(client: Client, source: ProjectSource) {
    let state: ProjectState;
    if (source.kind === "folder") {
      try {
        const adopted = store.adopt(source.dir.trim());
        state = addProject(adopted.project, adopted.threads);
      } catch (error) {
        toast(client, errorText(error));
        return;
      }
    } else {
      let next: { link: ResolveLink; local: LocalLink };
      if (source.kind === "open") {
        const open = openProject(resolve);
        if (!open) {
          toast(client, "Open a project in Resolve first.");
          return;
        }
        next = linkFromOpen(open);
      } else {
        if (!source.name.trim() || (source.kind === "cloud" && !source.mediaPath.trim())) {
          toast(client, source.kind === "cloud" ? "A cloud project needs its name and a media folder." : "Pick a project.");
          return;
        }
        next = linkFromSource(source);
      }
      const existing = [...projects.values()].find((project) => sameLink(next, project.saved));
      if (existing) {
        toast(client, `"${projectName(existing.saved)}" is already in Linker-Edit.`, "info");
        select(client, defaultThread(existing.saved.id));
        return;
      }
      state = addProject(store.create({ name: null, ...next }));
      state.history?.commitProjectFile(`Link to Resolve project "${next.link.name}"`).catch((error: unknown) => console.error(error));
    }
    select(client, defaultThread(state.saved.id));
  }

  function updateProject(projectId: string, changes: { name?: string; mediaPath?: string; syncMode?: CloudSyncMode }) {
    const project = projects.get(projectId);
    if (!project) return;
    const { saved } = project;
    const name = changes.name === undefined ? saved.name : changes.name.trim() || null;
    const local: LocalLink = {
      ...saved.local,
      ...(changes.mediaPath === undefined ? {} : { mediaPath: changes.mediaPath.trim() }),
      ...(changes.syncMode === undefined ? {} : { syncMode: changes.syncMode }),
    };
    project.saved = { ...saved, name, local };
    store.saveProject(project.saved);
    if (name !== saved.name) {
      project.history?.commitProjectFile(`Rename project to "${projectName(project.saved)}"`).catch((error: unknown) => console.error(error));
    }
    sendMeta();
  }

  function relink(client: Client, projectId: string) {
    const project = projects.get(projectId);
    const open = openProject(resolve);
    if (!project || !open) {
      if (project) toast(client, "Open the project in Resolve first, then link it.");
      return;
    }
    const owner = ownerOf(open);
    if (owner && owner !== project) {
      toast(client, `"${open.name}" already belongs to the project "${projectName(owner.saved)}".`);
      return;
    }
    const next = linkFromOpen(open);
    setLink(project, { link: next.link, local: { ...project.saved.local, ...next.local } }, `Link to Resolve project "${open.name}"`);
  }

  function removeProject(projectId: string) {
    const project = projects.get(projectId);
    if (!project) return;
    for (const thread of projectThreads(projectId)) {
      thread.dispose();
      threads.delete(thread.id);
    }
    projects.delete(projectId);
    store.removeProject(projectId);
    for (const [client, view] of clients) {
      if (view.threadId && !threads.has(view.threadId)) {
        clients.set(client, { threadId: defaultThread() });
        sendSnapshot(client);
      }
    }
    sendMeta();
  }

  async function openInResolve(client: Client, projectId: string) {
    const project = projects.get(projectId);
    const link = project?.saved.link;
    if (!project || !link || !options.resolveProjects) return;
    if (resolveBusy()) {
      toast(client, "Resolve is busy. Wait until the running turn or render finishes.");
      return;
    }
    opening = projectId;
    sendMeta();
    try {
      const opened = await options.resolveProjects.open(link, project.saved.local);
      const open: OpenProject = { name: opened.project.name, id: opened.project.id, database: opened.database };
      if (link.projectId && open.id && link.projectId !== open.id) {
        toast(client, `Resolve opened a different project named "${open.name}" than the one this project is linked to.`);
      } else {
        const next = refreshedLink(link, project.saved.local, open);
        setLink(project, { ...next, local: opened.folder ? { ...next.local, folder: opened.folder } : next.local }, `Link to Resolve project "${open.name}"`);
      }
    } catch (error) {
      toast(client, `Couldn't open "${link.name}" in Resolve: ${errorText(error)}`);
    } finally {
      opening = null;
      await refreshResolve().catch(() => {});
      sendMeta();
    }
  }

  async function loadLibrary(client: Client) {
    if (!options.resolveProjects) return;
    try {
      send(client, { type: "library", library: await options.resolveProjects.library() });
    } catch (error) {
      send(client, { type: "library", library: { error: errorText(error) } });
    }
  }

  function handleMessage(client: Client, message: ClientMessage) {
    const view = clients.get(client);
    const current = view?.threadId ? threads.get(view.threadId) : undefined;
    switch (message.type) {
      case "send":
        if (message.text.trim()) current?.thread.send(message.text.trim(), message.compactFirst === true);
        break;
      case "compact":
        current?.thread.compact();
        break;
      case "stop":
        current?.thread.stop();
        break;
      case "set_provider":
        lastProvider = message.provider;
        current?.thread.setProvider(message.provider);
        sendMeta();
        break;
      case "set_approval_mode":
        approvalMode = message.mode;
        if (approvalMode === "auto") for (const { thread } of threads.values()) thread.settleAllApprovals(true);
        sendMeta();
        break;
      case "approve":
        for (const { thread } of threads.values()) if (thread.approve(message.id, message.allow)) break;
        break;
      case "refresh_resolve":
        void refreshResolve();
        break;
      case "load_history":
        if (current) void publishHistory(current.projectId, client);
        break;
      case "render_preview":
        renderPreview(message.timelineId ?? null);
        break;
      case "stop_preview":
        previewJob?.stop();
        break;
      case "select_thread":
        if (threads.has(message.threadId)) select(client, message.threadId);
        break;
      case "new_thread":
        if (projects.has(message.projectId)) select(client, newThread(message.projectId).id);
        break;
      case "rename_thread":
        threads.get(message.threadId)?.thread.rename(message.title);
        break;
      case "archive_thread": {
        const entry = threads.get(message.threadId);
        if (!entry) break;
        if (message.archived && entry.thread.empty) removeThread(message.threadId);
        else {
          entry.thread.setArchived(message.archived);
          if (message.archived) moveViewers(message.threadId, entry.projectId);
        }
        break;
      }
      case "delete_thread":
        removeThread(message.threadId);
        break;
      case "add_project":
        addProjectFrom(client, message.source);
        break;
      case "update_project":
        updateProject(message.projectId, message);
        break;
      case "relink_project":
        relink(client, message.projectId);
        break;
      case "remove_project":
        removeProject(message.projectId);
        break;
      case "open_in_resolve":
        void openInResolve(client, message.projectId);
        break;
      case "load_library":
        void loadLibrary(client);
        break;
      case "refresh_remote_access":
        void options.remote?.refresh();
        break;
      case "set_remote_access":
        void options.remote?.setEnabled(message.enabled);
        break;
    }
  }

  return {
    connect(client: Client): () => void {
      clients.set(client, { threadId: defaultThread() });
      sendSnapshot(client);
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
    mediaFile(id: string) {
      for (const project of projects.values()) {
        const file = project.history?.mediaFile(id);
        if (file) return file;
      }
      return null;
    },
    previewFile: (name: string) => options.preview?.file(name) ?? null,
    playableTimeline: async (projectId: string | null, id: string | null, commit: string | null) => {
      const history = projectId ? projects.get(projectId)?.history : undefined;
      if (!history) throw new Error("This project has no edit history, so there is nothing to play.");
      return history.timeline(id, commit);
    },
    busy: resolveBusy,
    /** For the agents' MCP server: the transcripts as text, or what went wrong per file. */
    transcribe: async (files: string[], language?: string) => {
      if (!options.transcriber) throw new Error("Transcription is off.");
      return describeResults(await options.transcriber.transcribe(files, language));
    },
  };
}
