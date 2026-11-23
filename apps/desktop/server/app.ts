import { randomUUID } from "node:crypto";
import * as os from "node:os";
import * as path from "node:path";
import type {
  AppMeta,
  ApprovalMode,
  ComputerSetup,
  ChatItem,
  ClientMessage,
  LibraryClientMessage,
  LibraryFolder,
  LibraryQuery,
  LibraryScope,
  LibraryView,
  LinkerConnect,
  LocalLink,
  PreviewState,
  ProjectSidebarChanges,
  ProjectSource,
  ProjectSummary,
  Provider,
  ResolveLink,
  ResolveStatus,
  ServerMessage,
  SidebarClientMessage,
} from "../shared/protocol.ts";
import type { AppUpdates } from "./appUpdate.ts";
import { createCollections } from "./collections/collections.ts";
import type { CollectionStore } from "./collections/store.ts";
import { memoryCollectionStore } from "./collections/testing.ts";
import { ADD_LESSON, isBriefTool, SAVE_BRIEF, SAVE_EXAMPLE } from "./collections/tools.ts";
import { isStudyTool, type StudyService } from "./study/study.ts";
import { browseFolders, expandHome } from "./folders.ts";
import type { History, ProjectCheck } from "./history/history.ts";
import { checkFolder, cleanFolder, cleanTags, keepLibrary, newFolder, projectFolders, withProjectFolders } from "./library/folders.ts";
import type { LibraryLink, LibraryService } from "./library/library.ts";
import { libraryFile } from "./library/serve.ts";
import { sortFolder } from "./library/sort.ts";
import { describeFile, LIBRARY_IMPORT, LIBRARY_SEARCH, libraryImport, queryFromInput } from "./library/tools.ts";
import type { LinkerService } from "./linker.ts";
import type { ConnectService } from "./connect/connect.ts";
import { isLinkedProject, linkFromOpen, linkKind, openProject, refreshedLink, type OpenProject } from "./projects/link.ts";
import { NEW_THREAD_SIDEBAR, type ProjectStore, type SavedProject, type SavedThread } from "./projects/store.ts";
import { parseProjectIcon } from "../shared/project-icon.ts";
import type { PreviewRenderer } from "./preview/preview.ts";
import type { ProviderSession } from "./providers/types.ts";
import type { RemoteAccessService } from "./remote.ts";
import type { OpenOptions, ResolveProjects } from "./resolveProjects.ts";
import type { SettingsStore } from "./settings.ts";
import { findResolveTool } from "./resolve/tools.ts";
import { readOrigins, type Applied } from "./resolve/versions.ts";
import { LINKER_MCP_NAME, PROJECT_NOTES_FILE, readProjectFile, writeProjectFile } from "./rules.ts";
import { createThread, DEFAULT_TITLE, type Thread } from "./thread.ts";
import { describeResults, type Transcriber } from "./transcribe/transcriber.ts";

export interface AppOptions {
  version: string;
  store: ProjectStore;
  /**
   * resumeId continues a conversation from before a restart; dir is the project's working folder. rules gives
   * the agent's rules for the project as they are when a turn starts.
   */
  createSession: (
    provider: Provider,
    approvalMode: () => ApprovalMode,
    dir: string,
    resumeId: string | undefined,
    rules: () => string,
  ) => ProviderSession;
  checkResolve: () => Promise<ResolveStatus>;
  /** Each project's edit history in its folder; saving is off without it. */
  createHistory?: (dir: string, checkProject: ProjectCheck) => History;
  /** Renders timelines through Resolve for the preview panel. */
  preview?: PreviewRenderer;
  /** Lists and opens Resolve projects; without it projects can only follow what Resolve has open. */
  resolveProjects?: ResolveProjects;
  /** Gets a project folder ready for agents, such as its AGENTS.md with the project's rules. */
  prepareFolder?: (dir: string, rules: string) => void;
  /** Workspaces and series; kept in memory only without it. */
  collections?: CollectionStore;
  /** Runs one of Linker-Edit's own Resolve tools for an agent working in the project folder dir. */
  resolveTools?: (
    tool: string,
    input: Record<string, unknown>,
    dir: string,
    signal?: AbortSignal,
  ) => Promise<{ text: string; images?: string[] }>;
  /** Backs the open Resolve project up into the folder dir; where to. Agents change nothing without it. */
  backupProject?: (dir: string) => Promise<string>;
  /** Makes an approved AI timeline the main one, keeping the old one under another name. */
  applyTimeline?: (dir: string, timelineId: string) => Promise<Applied>;
  /** Speech to text for the agents' transcribe tool. */
  transcriber?: Transcriber;
  /** Opening the app from the user's other devices over Tailscale. */
  remote?: RemoteAccessService;
  /** The optional Linker account. */
  linker?: LinkerService;
  /** Linker Connect: this computer at its own address at Linker, for your other computers. */
  connect?: ConnectService;
  /** Choices kept across restarts, such as supervised mode. */
  settings?: SettingsStore;
  /** Where this computer keeps things, shown on the settings page. */
  setup?: ComputerSetup;
  /** The updates of the desktop app this server runs in; none with `npx linker-edit`. */
  appUpdates?: Pick<AppUpdates, "status" | "onStatus" | "check" | "install" | "cancel">;
  /** Folders of files the user reuses, for the library page and the agents' library tools. */
  library?: LibraryService;
  /** Measures example videos and compares a cut with its raw footage, for the agents' study tools. */
  study?: StudyService;
}

/** Starts the chat line that marks an approved edit; feedback rounds are counted from the last one. */
const APPROVED = "Approved:";
/** Starts the message Linker-Edit sends for the user after an approval, so it isn't counted as feedback. */
const LEARN = "Learn from this edit:";
/** The Resolve tools that work on files alone, which a setup thread may use. */
const usesNoResolve = (tool: string, input: Record<string, unknown>) =>
  tool === "find_silences" || (tool === "view_frames" && typeof input.file === "string");
const SETUP_REFUSAL = "This is a setup thread: it has no Resolve project, so tools that work in Resolve are refused here. Study the videos as files instead.";

const PROVIDER_NAME: Record<Provider, string> = { claude: "Claude", codex: "Codex" };

export interface Client {
  send(data: string): void;
  /** It applies "meta_patch" messages. The iPad app and pages of older releases don't, so they get whole metas. */
  metaPatches?: boolean;
}

export type App = ReturnType<typeof createApp>;

interface ProjectState {
  saved: SavedProject;
  history?: History;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** How many matching files the library page gets at once. */
const LIBRARY_PAGE = 300;

/** Linker Connect in builds and tests without it. */
const NO_CONNECT: LinkerConnect = {
  state: "signed_out",
  tunnel: true,
  url: null,
  connected: false,
  step: null,
  problem: null,
  computerId: "",
  computers: null,
  limit: 3,
  atLimit: false,
  devices: [],
};
const isLibraryMessage = (message: ClientMessage): message is LibraryClientMessage => message.type.startsWith("library_");

/** All app state lives here; clients get a snapshot of the thread they look at, and patches after. */
export function createApp(options: AppOptions) {
  const { store } = options;
  // Full access by default, like T3 Code: the rules are enforced in code, and the edit history undoes.
  // A switch to supervised is kept, so a restart never quietly turns it back into full access.
  let approvalMode: ApprovalMode = options.settings?.load().approvalMode ?? "auto";
  let resolve: ResolveStatus = { state: "checking" };
  let resolveHere = options.settings?.load().resolveHere ?? true;
  let preview: PreviewState = { state: "idle" };
  // The project the preview is of: other projects' threads see none. Resolve renders one at a time.
  let previewProject: string | null = null;
  let previewJob: { stop(): void } | null = null;
  let update: string | null = null;
  // Resolve has one project open and does one thing at a time: one turn, open or render app-wide.
  let turnOwner: string | null = null;
  let opening: string | null = null;
  // The project whose AI timeline is becoming its main one.
  let applying: string | null = null;
  // Agents' Resolve tool calls still running. A stopped turn ends before a render it started has wound
  // down, and Resolve stays taken until it has.
  let toolCalls = 0;
  // New threads start with the agent picked last.
  let lastProvider: Provider = options.settings?.load().defaultProvider ?? "claude";
  const projects = new Map<string, ProjectState>();
  const threads = new Map<string, { thread: Thread; projectId: string }>();
  const clients = new Map<Client, { threadId: string | null }>();
  const collections = createCollections({
    store: options.collections ?? memoryCollectionStore(),
    projects: () => [...projects.values()].map((project) => project.saved),
    setCollection(projectId, collection, commitMessage) {
      const project = projects.get(projectId);
      if (!project) return;
      project.saved = { ...project.saved, collection };
      store.saveProject(project.saved);
      project.history?.commitProjectFile(commitMessage).catch((error: unknown) => console.error("Couldn't commit linker-edit.json", error));
    },
    removeProject: (projectId) => removeProject(projectId),
    changed() {
      for (const project of projects.values()) prepareFolder(project.saved);
      sendMeta();
    },
  });

  function projectName(saved: SavedProject): string {
    // Follows a rename of what it sets up.
    if (saved.setup) return `${collections.get(saved.collection?.id ?? "")?.name ?? saved.collection?.name ?? "Brand"} setup`;
    return saved.name ?? saved.link?.name ?? path.basename(saved.dir);
  }
  const resolveBusy = () => turnOwner !== null || previewJob !== null || opening !== null || applying !== null || toolCalls > 0;

  /** Writes the project's rules where agents read them; again before each turn, so they are never stale. */
  function prepareFolder(saved: SavedProject) {
    try {
      options.prepareFolder?.(saved.dir, collections.rulesFor(saved));
    } catch (error) {
      console.error(error);
    }
  }

  function addProject(saved: SavedProject, savedThreads: SavedThread[] = []): ProjectState {
    prepareFolder(saved);
    const state: ProjectState = { saved };
    // A setup project has no Resolve project, so nothing to keep a history of.
    state.history = saved.setup ? undefined : options.createHistory?.(saved.dir, (open) => {
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
      createSession: (provider, resumeId) =>
        options.createSession(provider, () => approvalMode, project.saved.dir, resumeId, () => collections.rulesFor(project.saved)),
      history: () => project.history,
      backup: async () => {
        // Resolve's own run_script comes through here too: the last stop before any change.
        const refusal = await midTurnRefusal(project);
        if (refusal) throw new Error(refusal);
        return options.backupProject ? options.backupProject(project.saved.dir) : null;
      },
      claim() {
        if (resolveBusy()) return false;
        turnOwner = saved.id;
        return true;
      },
      release() {
        if (turnOwner === saved.id) turnOwner = null;
        // The agent may have written PROJECT.md in its turn.
        publishProjectFile(projectId);
      },
      check: () => {
        prepareFolder(project.saved);
        return checkBeforeTurn(project);
      },
      resolveConnected: () => resolve.state === "connected",
      shared: () => project.saved.link?.kind === "cloud",
      saved: async () => {
        if (preview.state === "ready" && previewProject === projectId) preview = { ...preview, outdated: true };
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
      ...NEW_THREAD_SIDEBAR,
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
    if (!open || project.saved.setup) return null;
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
    // Never linked to whatever Resolve has open: it studies files.
    if (project.saved.setup) return { notice: null };
    await refreshResolve();
    const refusal = projectRefusal(project);
    if (refusal) return { refusal };
    const open = openProject(resolve);
    if (!open || project.saved.link) return { notice: null };
    setLink(project, linkFromOpen(open), `Link to Resolve project "${open.name}"`);
    return { notice: `Linked this project to the Resolve project "${open.name}" in ${describeLibrary(project.saved.link!, open)}.` };
  }

  /**
   * Why a running turn can't use Resolve now; null when it can. A turn may start while Resolve is closed, so
   * which project it has open is checked once it's there.
   */
  async function midTurnRefusal(project: ProjectState): Promise<string | null> {
    if (project.saved.setup) return null;
    if (!openProject(resolve)) await refreshResolve();
    if (resolve.state === "not_running") return "Resolve isn't running. Start it with start_resolve.";
    if (resolve.state === "blocked") {
      return `${resolve.hint} If Resolve sits at its Project Manager, start_resolve opens this folder's project.`;
    }
    const open = openProject(resolve);
    // Only a message links a folder to the project open in Resolve, never an agent halfway through a turn.
    if (open && !project.saved.link) {
      return `This folder isn't linked to a Resolve project yet. Ask the user to send a message while the project they want is open in Resolve: now that's "${open.name}".`;
    }
    return projectRefusal(project);
  }

  /** The start_resolve tool: starts Resolve and opens the thread's own project, never another one. */
  async function startResolve(thread: Thread, project: ProjectState, signal: AbortSignal | undefined): Promise<string> {
    if (!options.resolveProjects) throw new Error("Linker-Edit can't start Resolve here.");
    const { link } = project.saved;
    if (!link) {
      await options.resolveProjects.start(signal);
      await refreshResolve();
      return "Resolve is running, but this folder isn't linked to a Resolve project yet. Ask the user to open or create the project in Resolve and send a message: that links it.";
    }
    try {
      await openLinked(project, link, { onlyIfIdle: true, signal });
    } finally {
      await refreshResolve();
    }
    const refusal = projectRefusal(project);
    if (refusal) throw new Error(refusal);
    // The turn began without Resolve, so the user's own changes are saved now, before the agent's.
    await thread.saveUserChanges();
    return `Resolve is running with "${project.saved.link?.name}" open.`;
  }

  function describeLibrary(link: ResolveLink, open: OpenProject): string {
    if (link.kind === "cloud") return "Blackmagic Cloud";
    const name = open.database?.name;
    return link.kind === "network" ? `the network database${name ? ` "${name}"` : ""}` : `the local database${name ? ` "${name}"` : ""}`;
  }

  function setLink(project: ProjectState, next: { link: ResolveLink | null; local: LocalLink }, commitMessage: string | null) {
    const linkChanged = JSON.stringify(next.link) !== JSON.stringify(project.saved.link);
    project.saved = { ...project.saved, link: next.link, local: keepLibrary(project.saved.local, next.local) };
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
      icon: saved.icon,
      pinned: saved.pinned,
      archived: saved.archived,
      threads: projectThreads(saved.id).map((thread) => thread.summary()),
      setup: saved.setup === true,
    }));
  }

  function blockedFor(thread: Thread, projectId: string): AppMeta["blocked"] {
    if (thread.busy) return null;
    if (opening) {
      const target = projects.get(opening);
      return { message: `Opening "${target ? projectName(target.saved) : "the project"}" in Resolve…`, canOpen: false };
    }
    if (applying) return { message: "Making the AI timeline the main one…", canOpen: false };
    if (previewJob) return { message: "Resolve is rendering a preview. Send when it's done, or stop it in the preview panel.", canOpen: false };
    const owner = turnOwner ? threads.get(turnOwner) : undefined;
    if (owner) {
      const where = owner.projectId === projectId ? "" : ` (${projectName(projects.get(owner.projectId)!.saved)})`;
      return {
        message: `${PROVIDER_NAME[owner.thread.provider]} is working in "${owner.thread.title}"${where}. One thread works in Resolve at a time.`,
        canOpen: false,
      };
    }
    if (toolCalls > 0) return { message: "Waiting for Resolve to finish the stopped turn's last step…", canOpen: false };
    const project = projects.get(projectId)!;
    const refusal = projectRefusal(project);
    return refusal ? { message: refusal, canOpen: !!project.saved.link && !!options.resolveProjects } : null;
  }

  /** The parts of the meta that are the same for every client, worked out once per broadcast. */
  function sharedMeta() {
    return {
      projects: summaries(),
      appUpdate: options.appUpdates?.status() ?? null,
      transcription: options.transcriber?.status() ?? { state: "idle" },
      remote: options.remote?.status() ?? { state: "unavailable", hint: "Remote access isn't available in this build." },
      connect: options.connect?.status() ?? NO_CONNECT,
      computer: os.hostname(),
      linker: options.linker?.status() ?? { state: "signed_out", problem: null },
      collections: collections.summaries(),
    } satisfies Partial<AppMeta>;
  }

  function metaFor(threadId: string | null, shared = sharedMeta()): AppMeta {
    const entry = threadId ? threads.get(threadId) : undefined;
    const project = entry ? projects.get(entry.projectId) : undefined;
    return {
      projects: shared.projects,
      threadId: entry ? threadId : null,
      projectId: entry?.projectId ?? null,
      provider: entry?.thread.provider ?? lastProvider,
      defaultProvider: lastProvider,
      approvalMode,
      busy: entry?.thread.busy ?? false,
      approvals: entry?.thread.approvals() ?? [],
      resolve,
      resolveHere,
      version: options.version,
      update,
      appUpdate: shared.appUpdate,
      workspaceDir: project?.saved.dir ?? null,
      preview: previewProject === (entry?.projectId ?? null) ? preview : { state: "idle" },
      context: entry?.thread.context ?? null,
      blocked: entry ? blockedFor(entry.thread, entry.projectId) : null,
      opening,
      applying,
      transcription: shared.transcription,
      remote: shared.remote,
      connect: shared.connect,
      computer: shared.computer,
      setup: options.setup ?? null,
      linker: shared.linker,
      collections: shared.collections,
    };
  }

  function send(client: Client, message: ServerMessage) {
    client.send(JSON.stringify(message));
  }

  // Each client's meta as last sent, field by field as JSON: a meta goes out only when something in it changed,
  // and a client that takes patches gets only the fields that did. The project list is most of a meta's size,
  // and a turn changes the rest (busy, context, approvals) many times without touching it.
  const sentMeta = new Map<Client, Map<string, string>>();

  /** The meta's fields as JSON; a value shared by several clients' metas is serialized once. */
  function metaFields(meta: AppMeta, cache: Map<unknown, string>): Map<string, string> {
    const fields = new Map<string, string>();
    for (const [key, value] of Object.entries(meta)) {
      if (value === undefined) continue;
      let json = typeof value === "object" && value !== null ? cache.get(value) : undefined;
      if (json === undefined) {
        json = JSON.stringify(value);
        if (typeof value === "object" && value !== null) cache.set(value, json);
      }
      fields.set(key, json);
    }
    return fields;
  }

  const jsonObject = (fields: Iterable<[string, string]>) => `{${[...fields].map(([key, json]) => `${JSON.stringify(key)}:${json}`).join(",")}}`;

  function sendMeta() {
    if (clients.size === 0) return;
    const shared = sharedMeta();
    const cache = new Map<unknown, string>();
    for (const [client, view] of clients) {
      const fields = metaFields(metaFor(view.threadId, shared), cache);
      const last = sentMeta.get(client);
      const changed = [...fields].filter(([key, json]) => last?.get(key) !== json);
      if (last && changed.length === 0) continue;
      sentMeta.set(client, fields);
      // The iPad app and pages of older releases replace their whole meta with each one.
      if (last && client.metaPatches) client.send(`{"type":"meta_patch","meta":${jsonObject(changed)}}`);
      else client.send(`{"type":"meta","meta":${jsonObject(fields)}}`);
    }
  }
  options.transcriber?.onStatus(sendMeta);
  options.remote?.onStatus(sendMeta);
  options.linker?.onStatus(sendMeta);
  options.connect?.onStatus(sendMeta);
  options.appUpdates?.onStatus(sendMeta);

  function sendSnapshot(client: Client) {
    const view = clients.get(client);
    if (!view) return;
    const items = view.threadId ? (threads.get(view.threadId)?.thread.items ?? []) : [];
    const fields = metaFields(metaFor(view.threadId), new Map());
    sentMeta.set(client, fields);
    client.send(`{"type":"snapshot","state":${jsonObject([...fields, ["items", JSON.stringify(items)]])}}`);
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

  /** A project's PROJECT.md as it is on disk. A setup project has none. */
  function publishProjectFile(projectId: string, client?: Client) {
    const saved = projects.get(projectId)?.saved;
    if (!saved || saved.setup) return;
    const message: ServerMessage = { type: "project_file", projectId, file: path.join(saved.dir, PROJECT_NOTES_FILE), text: readProjectFile(saved.dir) };
    for (const target of client ? [client] : clients.keys()) send(target, message);
  }

  function saveProjectFile(client: Client, projectId: string, text: unknown) {
    const saved = projects.get(projectId)?.saved;
    if (!saved || saved.setup || typeof text !== "string") return;
    const refusal = writeProjectFile(saved.dir, text);
    if (refusal) return toast(client, refusal);
    // Codex reads it from AGENTS.md, so that follows at once.
    prepareFolder(saved);
    publishProjectFile(projectId);
  }

  /**
   * The user approved an AI timeline: the project is backed up, then it takes the main timeline's name and
   * place, and the main one stays, renamed. The swap lands in the edit history.
   */
  async function applyTimeline(client: Client, projectId: string, timelineId: string) {
    const project = projects.get(projectId);
    if (!project || !options.applyTimeline || typeof timelineId !== "string" || !timelineId) return;
    if (resolveBusy()) return toast(client, "Resolve is busy with a turn, a render or an open. Try again when it's done.");
    applying = projectId;
    sendMeta();
    // The thread the user approved it from learns from the edit, when the project has a brief to learn into.
    const viewed = threads.get(clients.get(client)?.threadId ?? "");
    const learner = viewed?.projectId === projectId && collections.get(project.saved.collection?.id ?? "") ? viewed.thread : null;
    let learn: string | null = null;
    try {
      await refreshResolve();
      const refusal = projectRefusal(project);
      if (refusal) throw new Error(refusal);
      if (options.backupProject) await options.backupProject(project.saved.dir);
      const applied = await options.applyTimeline(project.saved.dir, timelineId);
      toast(
        null,
        `"${applied.was}" is now "${applied.main.name}". The earlier version is kept as "${applied.previous.name}".`,
        "info",
      );
      await project.history?.record().catch((error: unknown) => console.error("Couldn't save the edit history", error));
      await publishHistory(projectId);
      if (learner) {
        // Rounds of feedback: what the user sent since the last approval, less the brief that started the edit.
        const since = learner.items.slice(learner.items.findLastIndex((item) => item.kind === "notice" && item.text.startsWith(APPROVED)) + 1);
        const sent = since.filter((item) => item.kind === "user" && !item.text.startsWith(LEARN)).length;
        collections.recordEdit(project.saved, projectName(project.saved), Math.max(0, sent - 1));
        learner.note(`${APPROVED} "${applied.was}" is now the main timeline "${applied.main.name}".`);
        learn = `${LEARN} I approved "${applied.was}" and made it the main timeline "${applied.main.name}". Compare your first version of it with the approved one, and propose lessons for next time if there are any.`;
      }
    } catch (error) {
      toast(client, `Couldn't make it the main timeline: ${errorText(error)}`);
    } finally {
      applying = null;
      sendMeta();
      void refreshResolve();
    }
    if (learn && learner && !learner.busy) learner.send(learn, false);
  }

  function renderPreview(client: Client, projectId: string | null, timelineId: string | null) {
    if (!options.preview) return;
    if (resolveBusy()) return toast(client, "Resolve is busy with a turn, a render or an open. Render when it's done.");
    // Resolve renders whatever project it has open: only this thread's own.
    const project = projectId ? projects.get(projectId) : undefined;
    const refusal = project ? projectRefusal(project) : null;
    if (refusal) return toast(client, refusal);
    previewProject = projectId;
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

  /**
   * Downloads the speech engine in the background, so nobody has to ask for it and an agent's first
   * transcription doesn't wait on it. Not on a computer without Resolve: agents don't work there.
   */
  function prepareTranscription() {
    const status = options.transcriber?.status();
    if (!resolveHere || status?.state !== "idle" || status.ready !== false) return;
    options.transcriber?.setup().catch((error: unknown) => console.error(`Couldn't set up transcription: ${errorText(error)}`));
  }

  function setResolveHere(value: boolean) {
    if (value === resolveHere) return;
    resolveHere = value;
    options.settings?.save({ resolveHere });
    if (resolveHere) prepareTranscription();
    else options.transcriber?.cancelSetup();
    sendMeta();
  }

  let resolveCheck: Promise<void> | null = null;
  function refreshResolve(): Promise<void> {
    resolveCheck ??= options
      .checkResolve()
      .then((status) => {
        resolve = status;
        // Resolve turned up on a computer that was said to have none, so it's an edit computer after all.
        if (!resolveHere && status.state !== "setup_needed") setResolveHere(true);
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
        local: {},
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
        const adopted = store.adopt(expandHome(source.dir));
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
        if (!source.name.trim()) {
          toast(client, source.kind === "cloud" ? "A cloud project needs its name." : "Pick a project.");
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

  function updateProject(projectId: string, changes: { name?: string }) {
    const project = projects.get(projectId);
    if (!project) return;
    const { saved } = project;
    const name = changes.name === undefined ? saved.name : changes.name.trim() || null;
    project.saved = { ...saved, name };
    store.saveProject(project.saved);
    if (name !== saved.name) {
      project.history?.commitProjectFile(`Rename project to "${projectName(project.saved)}"`).catch((error: unknown) => console.error(error));
    }
    sendMeta();
  }

  function relink(client: Client, projectId: string) {
    const project = projects.get(projectId);
    const open = openProject(resolve);
    if (project?.saved.setup) return;
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

  async function openLinked(project: ProjectState, link: ResolveLink, openOptions?: OpenOptions) {
    const opened = await options.resolveProjects!.open(link, project.saved.local, openOptions);
    const open: OpenProject = { name: opened.project.name, id: opened.project.id, database: opened.database };
    if (link.projectId && open.id && link.projectId !== open.id) {
      throw new Error(`Resolve opened a different project named "${open.name}" than the one this project is linked to.`);
    }
    const next = refreshedLink(link, project.saved.local, open);
    setLink(project, { ...next, local: opened.folder ? { ...next.local, folder: opened.folder } : next.local }, `Link to Resolve project "${open.name}"`);
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
      await openLinked(project, link);
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

  async function sendFolders(client: Client, requestId: number, dir: string) {
    try {
      send(client, { type: "folders", requestId, listing: await browseFolders(dir) });
    } catch (error) {
      send(client, { type: "folders", requestId, listing: { error: errorText(error) } });
    }
  }

  /** The thread whose turn may use Resolve now, or why none may. */
  function runningTurn(): { thread: Thread; projectId: string; dir: string; signal: AbortSignal | undefined } | { refusal: string } {
    const entry = turnOwner ? threads.get(turnOwner) : undefined;
    const dir = entry ? projects.get(entry.projectId)?.saved.dir : undefined;
    if (!entry?.thread.busy || !dir) return { refusal: "No Linker-Edit turn is running, so Resolve calls are refused." };
    const signal = entry.thread.signal ?? undefined;
    if (signal?.aborted) return { refusal: "The user stopped this turn." };
    return { thread: entry.thread, projectId: entry.projectId, dir, signal };
  }

  /**
   * Before an agent changes Resolve: in "ask" mode the change waits for the user, for every agent alike, and the
   * project is backed up first. A refusal's text, or null when it may go on.
   */
  async function allowChange(thread: Thread, tool: string, input: Record<string, unknown>): Promise<string | null> {
    if (approvalMode === "ask" && !(await thread.requestApproval(`mcp__${LINKER_MCP_NAME}__${tool}`, input))) {
      return "The user declined this action.";
    }
    try {
      await thread.backup();
      return null;
    } catch (error) {
      return errorText(error);
    }
  }

  /** Keeps Resolve taken while an agent's call runs: a stopped turn ends before its last call has wound down. */
  async function holdingResolve<T>(run: () => Promise<T>): Promise<T> {
    toolCalls += 1;
    try {
      return await run();
    } finally {
      toolCalls -= 1;
      if (toolCalls === 0) sendMeta();
    }
  }

  // --- Library ---

  // What each client's library page shows, so it's sent again whenever the library changes.
  const libraryViews = new Map<Client, { scope: LibraryScope; query: LibraryQuery }>();
  options.library?.onChange(sendLibraryViews);

  /** The libraries a project gets, least specific first. Workspaces and series will add theirs in between. */
  function libraryChain(projectId: string | null): LibraryLink[] {
    const chain: LibraryLink[] = [{ scope: { kind: "universal" }, label: "Universal", folders: options.library?.folders() ?? [] }];
    const project = projectId ? projects.get(projectId) : undefined;
    if (project) {
      chain.push({ scope: { kind: "project", projectId: project.saved.id }, label: projectName(project.saved), folders: projectFolders(project.saved.local) });
    }
    return chain;
  }

  function allLibraryFolders(): LibraryFolder[] {
    return [...(options.library?.folders() ?? []), ...[...projects.values()].flatMap((project) => projectFolders(project.saved.local))];
  }

  function libraryLink(scope: LibraryScope | undefined): LibraryLink | null {
    if (scope?.kind === "universal") return libraryChain(null)[0]!;
    if (scope?.kind === "project" && projects.has(scope.projectId)) return libraryChain(scope.projectId).at(-1)!;
    return null;
  }

  function saveLibraryFolders(scope: LibraryScope, folders: LibraryFolder[]) {
    if (scope.kind === "universal") {
      options.library?.saveFolders(folders);
      return;
    }
    const project = projects.get(scope.projectId);
    if (!project) return;
    project.saved = { ...project.saved, local: withProjectFolders(project.saved.local, folders) };
    store.saveProject(project.saved);
  }

  function libraryView(client: Client, wanted: { scope: LibraryScope; query: LibraryQuery }): LibraryView {
    const current = clients.get(client)?.threadId;
    const projectId = current ? (threads.get(current)?.projectId ?? null) : null;
    const link = libraryLink(wanted.scope) ?? libraryChain(null)[0]!;
    const scopes = libraryChain(projectId).map(({ scope, label }) => ({ scope, label }));
    if (link.scope.kind === "project" && !scopes.some((entry) => JSON.stringify(entry.scope) === JSON.stringify(link.scope))) {
      scopes.push({ scope: link.scope, label: link.label });
    }
    const files = options.library?.search([link], wanted.query) ?? [];
    return {
      scope: link.scope,
      scopes,
      folders: link.folders.map((folder) => options.library!.status(folder)),
      files: files.slice(0, LIBRARY_PAGE),
      total: files.length,
    };
  }

  function sendLibraryViews() {
    for (const [client, wanted] of libraryViews) send(client, { type: "library_view", view: libraryView(client, wanted) });
  }

  /** The library page's messages: what it shows, and changes to folders and files. Files on disk are never touched. */
  function libraryMessage(client: Client, message: LibraryClientMessage) {
    const library = options.library;
    if (!library) return;
    try {
      switch (message.type) {
        case "library_open":
          libraryViews.set(client, { scope: message.scope, query: message.query && typeof message.query === "object" ? message.query : {} });
          break;
        case "library_close":
          libraryViews.delete(client);
          return;
        case "library_add_folder": {
          const link = libraryLink(message.scope);
          if (!link) return;
          const folder = newFolder({ ...message.folder, path: expandHome(String(message.folder?.path ?? "")) }, link.folders);
          saveLibraryFolders(link.scope, [...link.folders, folder]);
          void library.scan(folder);
          break;
        }
        case "library_add_folders": {
          const link = libraryLink(message.scope);
          if (!link || !Array.isArray(message.folders)) return;
          const added: LibraryFolder[] = [];
          const problems: string[] = [];
          for (const fields of message.folders.slice(0, 200)) {
            try {
              added.push(newFolder({ ...fields, path: expandHome(String(fields?.path ?? "")) }, [...link.folders, ...added]));
            } catch (error) {
              problems.push(errorText(error));
            }
          }
          if (added.length) saveLibraryFolders(link.scope, [...link.folders, ...added]);
          for (const folder of added) void library.scan(folder);
          if (problems.length) toast(client, problems.length === 1 ? problems[0]! : `${problems.length} folders weren't added: ${problems.join(" ")}`);
          break;
        }
        case "library_sort_folder": {
          const requestId = message.requestId;
          sortFolder(expandHome(String(message.path ?? "")))
            .then((sort) => send(client, { type: "library_sorted", requestId, sort }))
            .catch((error: unknown) => send(client, { type: "library_sorted", requestId, sort: { error: errorText(error) } }));
          return;
        }
        case "library_update_folder": {
          const link = libraryLink(message.scope);
          const old = link?.folders.find((folder) => folder.id === message.folderId);
          if (!link || !old) return;
          const changes = message.changes ?? {};
          const next = cleanFolder({ ...old, ...changes, id: old.id, path: changes.path === undefined ? old.path : expandHome(String(changes.path)) });
          if (!next) throw new Error("Pick a kind and a folder.");
          if (next.path !== old.path) checkFolder(next, link.folders);
          saveLibraryFolders(link.scope, link.folders.map((folder) => (folder.id === old.id ? next : folder)));
          if (next.path !== old.path || next.kind !== old.kind) {
            library.forget(old);
            void library.scan(next);
          }
          break;
        }
        case "library_remove_folder": {
          const link = libraryLink(message.scope);
          const old = link?.folders.find((folder) => folder.id === message.folderId);
          if (!link || !old) return;
          saveLibraryFolders(link.scope, link.folders.filter((folder) => folder.id !== old.id));
          library.forget(old);
          break;
        }
        case "library_rescan": {
          const folder = allLibraryFolders().find((candidate) => candidate.id === message.folderId);
          if (folder) void library.scan(folder);
          return;
        }
        case "library_update_file": {
          const folder = allLibraryFolders().find((candidate) => candidate.id === message.folderId);
          const parts = typeof message.rel === "string" ? message.rel.split("/") : [];
          if (!folder || !parts.length || parts.some((part) => !part || part === "." || part === "..")) return;
          library.updateFile(path.join(folder.path, ...parts), {
            ...(typeof message.star === "boolean" ? { star: message.star } : {}),
            ...(message.tags === undefined ? {} : { tags: cleanTags(message.tags) }),
            ...(typeof message.note === "string" ? { note: message.note.trim().slice(0, 2000) } : {}),
          });
          return;
        }
      }
    } catch (error) {
      toast(client, errorText(error));
    }
    sendLibraryViews();
  }

  async function libraryTool(tool: string, input: Record<string, unknown>): Promise<{ text: string; isError: boolean }> {
    const library = options.library;
    if (!library) return { text: "The library is off in this Linker-Edit.", isError: true };
    const turn = runningTurn();
    // A search only reads, so it works between turns too, in the universal library.
    const chain = libraryChain("refusal" in turn ? null : turn.projectId);
    if (tool === LIBRARY_SEARCH) {
      const limit = typeof input.limit === "number" && input.limit > 0 ? Math.min(Math.floor(input.limit), 100) : 30;
      const found = library.search(chain, queryFromInput(input));
      const folders = new Map(chain.flatMap((link) => link.folders).map((folder) => [folder.id, folder]));
      const scanning = [...folders.values()].filter((folder) => library.status(folder).scanning).map((folder) => `"${folder.label}"`);
      const notes = [
        ...(folders.size === 0 ? ["The library has no folders yet. The user adds them on Linker-Edit's library page."] : []),
        ...(scanning.length ? [`Still reading ${scanning.join(", ")}: more files may show up in a minute.`] : []),
        ...(found.length > limit ? [`${found.length - limit} more match. Narrow it down, or raise limit.`] : []),
      ];
      const results = found.slice(0, limit).map((file) => describeFile(file, folders.get(file.folderId)!));
      return { text: JSON.stringify({ results, total: found.length, ...(notes.length ? { note: notes.join(" ") } : {}) }), isError: false };
    }
    if (tool !== LIBRARY_IMPORT) return { text: `There is no library tool called ${tool}.`, isError: true };
    const run = options.resolveTools;
    if ("refusal" in turn) return { text: turn.refusal, isError: true };
    if (!run) return { text: "Linker-Edit can't reach Resolve's tools here.", isError: true };
    const project = projects.get(turn.projectId)!;
    if (project.saved.setup) return { text: SETUP_REFUSAL, isError: true };
    try {
      const result = await holdingResolve(() =>
        libraryImport(input, {
          find: (file) => library.find(chain, file),
          resolve: async (name, args) => (await run(name, args, turn.dir, turn.signal)).text,
          allowChange: (args) => allowChange(turn.thread, LIBRARY_IMPORT, args),
          recordUse: (file, use) => library.recordUse(file, { ...use, at: new Date().toISOString() }),
          project: { id: project.saved.id, name: projectName(project.saved) },
          currentTimeline: resolve.state === "connected" ? resolve.timeline : null,
        }),
      );
      return { text: JSON.stringify(result), isError: false };
    } catch (error) {
      return { text: errorText(error), isError: true };
    }
  }

  /** The agents' study tools: they only read media files, so they need no turn and no Resolve. */
  async function studyTool(tool: string, input: Record<string, unknown>): Promise<{ text: string; isError: boolean; images?: string[] }> {
    if (!options.study || !isStudyTool(tool)) return { text: `There is no study tool called ${tool}.`, isError: true };
    const turn = runningTurn();
    try {
      return { ...(await options.study.run(tool, input, "refusal" in turn ? undefined : turn.signal)), isError: false };
    } catch (error) {
      return { text: errorText(error), isError: true };
    }
  }

  /**
   * The agents' brief tools. New rules and lessons steer every later edit, so they wait for the user in full
   * access too; the agent's own notes on an example don't.
   */
  async function briefTool(tool: string, input: Record<string, unknown>): Promise<{ text: string; isError: boolean }> {
    if (!isBriefTool(tool)) return { text: `There is no brief tool called ${tool}.`, isError: true };
    const turn = runningTurn();
    if ("refusal" in turn) return { text: "No Linker-Edit turn is running, so the brief can't be changed.", isError: true };
    const project = projects.get(turn.projectId)!.saved;
    try {
      if (tool === SAVE_EXAMPLE) return { text: collections.saveExample(project, input), isError: false };
      const proposal = tool === SAVE_BRIEF ? collections.proposeBrief(project, input) : collections.proposeLesson(project, input, projectName(project));
      if ("error" in proposal) return { text: proposal.error, isError: true };
      if (!(await turn.thread.requestApproval(`mcp__${LINKER_MCP_NAME}__${tool}`, proposal.shows))) {
        return { text: "The user declined this change. Ask what they'd like instead; don't propose the same again.", isError: true };
      }
      return { text: proposal.apply(), isError: false };
    } catch (error) {
      return { text: errorText(error), isError: true };
    }
  }

  /** A workspace's or series' setup thread: its newest one, in a project made for it on first use. */
  function openSetup(client: Client, collectionId: string) {
    const collection = collections.get(collectionId);
    if (!collection) return;
    const existing = [...projects.values()].find((project) => project.saved.setup && project.saved.collection?.id === collectionId);
    const project =
      existing ??
      addProject(store.create({ name: `${collection.name} setup`, link: null, local: {}, collection: { id: collection.id, name: collection.name }, setup: true }));
    select(client, projectThreads(project.saved.id).find((thread) => !thread.archived)?.id ?? newThread(project.saved.id).id);
  }

  /** Workspaces and series: create, update, delete, and moving a project into one. */
  function handleCollectionMessage(client: Client, message: ClientMessage) {
    const refusal = collections.handle(message);
    if (refusal) toast(client, refusal);
  }

  function handleMessage(client: Client, message: ClientMessage) {
    if (isLibraryMessage(message)) return libraryMessage(client, message);
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
        if (!(message.provider in PROVIDER_NAME)) break;
        lastProvider = message.provider;
        options.settings?.save({ defaultProvider: lastProvider });
        current?.thread.setProvider(message.provider);
        sendMeta();
        break;
      case "set_default_provider":
        if (!(message.provider in PROVIDER_NAME)) break;
        lastProvider = message.provider;
        options.settings?.save({ defaultProvider: lastProvider });
        sendMeta();
        break;
      case "set_approval_mode":
        // Anything but "ask" would let every call run.
        if (message.mode !== "ask" && message.mode !== "auto") break;
        approvalMode = message.mode;
        options.settings?.save({ approvalMode });
        // Changes to the brief keep waiting: those ask in full access too.
        if (approvalMode === "auto") {
          for (const { thread } of threads.values()) thread.settleAllApprovals(true, (request) => !isBriefTool(request.tool.split("__").at(-1) ?? ""));
        }
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
      case "load_project_file":
        if (current) publishProjectFile(current.projectId, client);
        break;
      case "save_project_file":
        saveProjectFile(client, message.projectId, message.text);
        break;
      case "render_preview":
        renderPreview(client, current?.projectId ?? null, message.timelineId ?? null);
        break;
      case "apply_timeline":
        void applyTimeline(client, message.projectId, message.timelineId);
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
        updateProjectSidebar(message.projectId, message);
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
      case "browse_folders":
        if (typeof message.path === "string") void sendFolders(client, message.requestId, message.path);
        break;
      case "refresh_remote_access":
        void options.remote?.refresh();
        break;
      case "set_remote_access":
        void options.remote?.setEnabled(message.enabled);
        break;
      case "refresh_connect":
        void options.connect?.refresh();
        break;
      case "set_connect":
        if (typeof message.enabled === "boolean") void options.connect?.setEnabled(message.enabled);
        break;
      case "set_connect_tunnel":
        if (typeof message.tunnel === "boolean") void options.connect?.setTunnel(message.tunnel);
        break;
      case "connect_unlink":
        if (typeof message.computerId === "string" && message.computerId) void options.connect?.unlink(message.computerId);
        break;
      case "connect_sign_out_device":
        if (typeof message.deviceId === "string") options.connect?.signOutDevice(message.deviceId);
        break;
      case "linker_cancel_sign_in":
        options.linker?.cancel();
        break;
      case "linker_sign_out":
        void options.linker?.signOut();
        break;
      case "set_resolve_here":
        if (typeof message.resolveHere === "boolean") setResolveHere(message.resolveHere);
        break;
      case "transcription_setup":
        options.transcriber?.setup().then(
          () => toast(client, "Transcription is set up.", "info"),
          (error: unknown) => toast(client, `Couldn't set up transcription: ${errorText(error)}`),
        );
        break;
      case "transcription_cancel":
        options.transcriber?.cancelSetup();
        break;
      case "check_app_update":
        options.appUpdates?.check();
        break;
      case "install_app_update":
        options.appUpdates?.install();
        break;
      case "cancel_app_update":
        options.appUpdates?.cancel();
        break;
      case "setup_collection":
        if (typeof message.id === "string") openSetup(client, message.id);
        break;
      default:
        if (!handleSidebarMessage(message)) handleCollectionMessage(client, message);
    }
  }

  // --- Sidebar (T3 Code) ---

  const SIDEBAR_MESSAGES = new Set<string>(["pin_thread", "settle_thread", "snooze_thread", "mark_read"] satisfies SidebarClientMessage["type"][]);
  const isSidebarMessage = (message: ClientMessage): message is SidebarClientMessage => SIDEBAR_MESSAGES.has(message.type);

  /** False when the message isn't the sidebar's. */
  function handleSidebarMessage(message: ClientMessage): boolean {
    if (!isSidebarMessage(message)) return false;
    const thread = threads.get(message.threadId)?.thread;
    switch (message.type) {
      case "pin_thread":
        thread?.setPinned(message.pinned === true);
        break;
      case "settle_thread":
        thread?.setSettled(message.settled === true);
        break;
      case "snooze_thread":
        if (message.until === null) thread?.snooze(null);
        else if (typeof message.until === "string" && Date.parse(message.until) > Date.now()) thread?.snooze(new Date(message.until).toISOString());
        break;
      case "mark_read":
        thread?.markRead(message.read !== false);
        break;
    }
    return true;
  }

  function updateProjectSidebar(projectId: string, changes: ProjectSidebarChanges) {
    const project = projects.get(projectId);
    if (!project || (changes.icon === undefined && changes.pinned === undefined && changes.archived === undefined)) return;
    const { saved } = project;
    const icon = changes.icon === undefined ? saved.icon : parseProjectIcon(changes.icon);
    project.saved = {
      ...saved,
      icon,
      pinned: typeof changes.pinned === "boolean" ? changes.pinned : saved.pinned,
      archived: typeof changes.archived === "boolean" ? changes.archived : saved.archived,
    };
    store.saveProject(project.saved);
    if (JSON.stringify(icon) !== JSON.stringify(saved.icon)) {
      project.history?.commitProjectFile("Change project icon").catch((error: unknown) => console.error(error));
    }
    sendMeta();
  }

  return {
    version: options.version,
    connect(client: Client): () => void {
      clients.set(client, { threadId: defaultThread() });
      sendSnapshot(client);
      return () => {
        clients.delete(client);
        sentMeta.delete(client);
        libraryViews.delete(client);
      };
    },
    handleMessage,
    refreshResolve,
    prepareTranscription,
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
      const project = projectId ? projects.get(projectId) : undefined;
      if (!project?.history) throw new Error("This project has no edit history, so there is nothing to play.");
      // The live timeline comes from the project open in Resolve, which must be this one.
      const refusal = commit ? null : projectRefusal(project);
      if (refusal) throw new Error(refusal);
      const timeline = await project.history.timeline(id, commit);
      // As Resolve has it now, an AI timeline says which main timeline it can replace.
      const replaces = timeline && !commit ? readOrigins(project.saved.dir)[timeline.id] : undefined;
      return timeline && replaces ? { ...timeline, replaces } : timeline;
    },
    busy: resolveBusy,
    /**
     * One of Linker-Edit's Resolve tools, called by an agent's "linker" MCP server. Only the thread with a
     * turn running may use Resolve, and in "ask" mode changes wait for the user, for every agent alike.
     */
    async resolveTool(tool: string, input: Record<string, unknown>): Promise<{ text: string; isError: boolean; images?: string[] }> {
      const definition = findResolveTool(tool);
      const run = options.resolveTools;
      if (!definition || !run) return { text: `There is no Resolve tool called ${tool}.`, isError: true };
      const turn = runningTurn();
      if ("refusal" in turn) return { text: turn.refusal, isError: true };
      if (projects.get(turn.projectId)?.saved.setup && !usesNoResolve(tool, input)) return { text: SETUP_REFUSAL, isError: true };
      const project = projects.get(turn.projectId)!;
      try {
        if (tool === "start_resolve") {
          return { text: await holdingResolve(() => startResolve(turn.thread, project, turn.signal)), isError: false };
        }
        const wrong = usesNoResolve(tool, input) ? null : await midTurnRefusal(project);
        if (wrong) return { text: wrong, isError: true };
        if (definition.kind === "write") {
          const refusal = await allowChange(turn.thread, tool, input);
          if (refusal) return { text: refusal, isError: true };
        }
        return { ...(await holdingResolve(() => run(tool, input, turn.dir, turn.signal))), isError: false };
      } catch (error) {
        return { text: errorText(error), isError: true };
      }
    },
    /** One of the library's tools, called by an agent's "linker" MCP server. */
    libraryTool,
    studyTool,
    briefTool,
    /** A library file for the library page to play, or null when it isn't one; see library/serve.ts. */
    libraryFile: (folderId: string, rel: string) => libraryFile(allLibraryFolders(), folderId, rel),
    /** Rescans every library folder, so files added or changed while the app was closed show up. */
    scanLibrary: () => options.library?.scanAll(allLibraryFolders()) ?? Promise.resolve(),
    /** For the agents' MCP server: the transcripts as text, or what went wrong per file. */
    transcribe: async (files: string[], language?: string) => {
      if (!options.transcriber) throw new Error("Transcription is off.");
      return describeResults(await options.transcriber.transcribe(files, language));
    },
  };
}
