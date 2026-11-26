import { randomUUID } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { parseProjectIcon } from "../../shared/project-icon.ts";
import type { ChatItem, ContextUsage, LocalLink, ProjectIcon, Provider, ResolveLink, ThreadSidebarState } from "../../shared/protocol.ts";

/** A thread as kept on disk, so a restart continues it. The sidebar fields are null in files from before them. */
export interface SavedThread extends Omit<ThreadSidebarState, "createdAt"> {
  id: string;
  title: string;
  provider: Provider;
  /** The provider's conversation id; null before the first turn. */
  resumeId: string | null;
  items: ChatItem[];
  context: ContextUsage | null;
  /** ISO 8601. */
  createdAt: string;
  updatedAt: string;
  archivedAt: string | null;
}

export interface SavedProject {
  /** This machine's id for the project; it isn't in the project's own files. */
  id: string;
  dir: string;
  /** Null shows the linked Resolve project's name. */
  name: string | null;
  link: ResolveLink | null;
  local: LocalLink;
  /** In linker-edit.json, so a teammate sees the same icon. */
  icon: ProjectIcon | null;
  /** This machine's own sidebar choices, in projects.json. */
  pinned: boolean;
  archived: boolean;
  /** The workspace or series it is in. Missing in files from before workspaces. */
  collection?: ProjectCollection | null;
  /** Holds its collection's setup threads rather than a Resolve project; see ProjectSummary.setup. */
  setup?: boolean;
  /** Whether its agent makes the first cut without asking; missing follows its series or workspace. */
  autonomous?: boolean;
  /** Its folder on Google Drive; missing for a project that isn't on Drive. */
  drive?: DriveLink;
  /** This computer keeps a copy of the project on Drive and syncs it both ways. */
  mirrored?: boolean;
}

/** In linker-edit.json, so every computer syncs the same Drive folder. */
export interface DriveLink {
  folderId: string;
  /** Drive's id for the Google account, so a computer signed in to another account leaves it alone. */
  accountId: string;
}

/** The sidebar fields of a thread that has none yet. */
export const NEW_THREAD_SIDEBAR: Omit<ThreadSidebarState, "createdAt"> = {
  pinnedAt: null,
  settledAt: null,
  unsettledAt: null,
  snoozedUntil: null,
  snoozedAt: null,
  lastReadAt: null,
  lastSentAt: null,
  turnEndedAt: null,
};

/** A saved thread with every sidebar field set, whatever version of Linker-Edit wrote it. */
function withSidebarFields(thread: SavedThread): SavedThread {
  const loaded = { ...thread };
  for (const key of Object.keys(NEW_THREAD_SIDEBAR) as (keyof typeof NEW_THREAD_SIDEBAR)[]) {
    const value = loaded[key];
    loaded[key] = typeof value === "string" && !Number.isNaN(Date.parse(value)) ? value : null;
  }
  return loaded;
}

/** In linker-edit.json, so a teammate who gets the project sees which workspace or series it belongs to. */
export interface ProjectCollection {
  id: string;
  /** As it was when the project was put in it, for computers that don't have it. */
  name: string;
}

export interface ProjectStore {
  /** Every project with its threads, in the order they were added. */
  load(): { project: SavedProject; threads: SavedThread[] }[];
  /** A new folder under the store's root for a new project. */
  create(project: Omit<SavedProject, "id" | "dir" | "icon" | "pinned" | "archived">): SavedProject;
  /** Registers a folder that already has a linker-edit.json, with the threads it has. */
  adopt(dir: string): { project: SavedProject; threads: SavedThread[] };
  saveProject(project: SavedProject): void;
  /** The project as its files say now, after another computer's linker-edit.json came down from Drive. */
  reload(project: SavedProject): SavedProject;
  /** Only forgets the project; its folder stays. */
  removeProject(id: string): void;
  saveThread(project: SavedProject, thread: SavedThread): void;
  deleteThread(project: SavedProject, threadId: string): void;
}

/** The committed file that says which Resolve project a folder belongs to. */
export const PROJECT_FILE = "linker-edit.json";
export const STATE_DIR = ".linker-edit";

// Bump when a file changes shape; a file with another version is ignored rather than misread.
const VERSION = 1;

interface ProjectFile {
  version: number;
  name: string | null;
  resolve: ResolveLink | null;
  /** Missing in files from before project icons. */
  icon?: ProjectIcon | null;
  collection?: ProjectCollection;
  setup?: boolean;
  autonomous?: boolean;
  drive?: DriveLink;
}

/** This computer's side of a project: its link's local half, and the plan's Mirror for a project on Drive. */
type LocalJson = LocalLink & { mirror?: { state: "mirrored" } };

interface RegistryEntry {
  id: string;
  dir: string;
  /** Missing in files from before the T3 Code sidebar. */
  pinned?: boolean;
  archived?: boolean;
}

interface Registry {
  version: number;
  projects: RegistryEntry[];
}

/** The one chat Linker-Edit kept before it had threads, in <workspace>/.linker-edit/chat.json. */
interface LegacyChat {
  provider: Provider;
  resumeId: string | null;
  items: ChatItem[];
  context: ContextUsage | null;
}

export function threadTitle(items: ChatItem[]): string {
  const first = items.find((item) => item.kind === "user");
  const line = first?.text.trim().split("\n")[0]?.trim() ?? "";
  return line ? (line.length > 80 ? `${line.slice(0, 79)}…` : line) : "New thread";
}

// Windows can't name a folder with these, or with a trailing dot or space.
function folderName(name: string): string {
  const cleaned = name
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, " ")
    .replace(/\s+/g, " ")
    .trim()
    .replace(/[. ]+$/, "");
  return cleaned.slice(0, 60) || "Project";
}

function readJson<T>(file: string): T | null {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8")) as T;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw error;
  }
}

// Written whole and renamed, so a crash mid-write can't leave half a file.
function writeJson(file: string, value: unknown) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const temp = `${file}.tmp`;
  fs.writeFileSync(temp, `${JSON.stringify(value, null, 2)}\n`);
  fs.renameSync(temp, file);
}

/**
 * Projects registered in <root>/projects.json, each in its own folder. On the first run the old
 * single workspace becomes the first project, in place, and its chat its first thread.
 */
export function fileProjectStore(options: { root: string; legacyDir: string; onError?: (error: unknown) => void }): ProjectStore {
  const { root } = options;
  const onError = options.onError ?? console.error;
  const registryFile = path.join(root, "projects.json");
  const stateDir = (dir: string) => path.join(dir, STATE_DIR);
  const threadsDir = (dir: string) => path.join(stateDir(dir), "threads");
  const threadFile = (dir: string, id: string) => path.join(threadsDir(dir), `${id}.json`);

  let registry: Registry = readRegistry();

  function readRegistry(): Registry {
    let problem: string | null = null;
    try {
      const saved = readJson<Registry>(registryFile);
      if (saved?.version === VERSION && Array.isArray(saved.projects)) return saved;
      if (saved) problem = "is from another Linker-Edit version";
    } catch (error) {
      problem = `can't be read (${error instanceof Error ? error.message : String(error)})`;
    }
    if (problem) {
      // Kept, not overwritten: the list is rebuilt from the project folders, and the old file stays to look at.
      const kept = `${registryFile}.bad-${Date.now()}`;
      try {
        fs.renameSync(registryFile, kept);
      } catch (error) {
        onError(error);
      }
      onError(new Error(`${registryFile} ${problem}. It was kept as ${kept}, and the project list was rebuilt from the folders in ${root}.`));
    }
    const rebuilt: Registry = {
      version: VERSION,
      projects: [options.legacyDir, ...(problem ? projectFolders() : [])]
        .filter((dir, index, dirs) => dirs.findIndex((other) => path.resolve(other) === path.resolve(dir)) === index)
        .map((dir) => ({ id: randomUUID(), dir })),
    };
    fs.mkdirSync(options.legacyDir, { recursive: true });
    writeJson(registryFile, rebuilt);
    return rebuilt;
  }

  /** The project folders under the root, found by their linker-edit.json. */
  function projectFolders(): string[] {
    try {
      return fs
        .readdirSync(root, { withFileTypes: true })
        .filter((entry) => entry.isDirectory() && fs.existsSync(path.join(root, entry.name, PROJECT_FILE)))
        .map((entry) => path.join(root, entry.name));
    } catch {
      return [];
    }
  }

  function saveRegistry() {
    try {
      writeJson(registryFile, registry);
    } catch (error) {
      onError(error);
    }
  }

  function prepare(dir: string) {
    fs.mkdirSync(stateDir(dir), { recursive: true });
    fs.writeFileSync(path.join(stateDir(dir), ".gitignore"), "*\n");
  }

  function readProject({ id, dir, pinned, archived }: RegistryEntry): SavedProject {
    let file: ProjectFile | null = null;
    let saved: LocalJson = {};
    try {
      file = readJson<ProjectFile>(path.join(dir, PROJECT_FILE));
      if (file && file.version !== VERSION) file = null;
      saved = readJson<LocalJson>(path.join(stateDir(dir), "local.json")) ?? {};
    } catch (error) {
      onError(error);
    }
    const { mirror, ...local } = saved;
    // Files from before workspaces have none.
    const collection = file?.collection;
    const drive = file?.drive;
    return {
      id,
      dir,
      name: file?.name ?? null,
      link: file?.resolve ?? null,
      local,
      icon: parseProjectIcon(file?.icon),
      pinned: pinned === true,
      archived: archived === true,
      ...(collection && typeof collection.id === "string" ? { collection: { id: collection.id, name: String(collection.name ?? "") } } : {}),
      ...(file?.setup === true ? { setup: true } : {}),
      ...(typeof file?.autonomous === "boolean" ? { autonomous: file.autonomous } : {}),
      ...(typeof drive?.folderId === "string" && typeof drive.accountId === "string" ? { drive: { folderId: drive.folderId, accountId: drive.accountId } } : {}),
      ...(mirror?.state === "mirrored" ? { mirrored: true } : {}),
    };
  }

  function migrateLegacyChat(dir: string): SavedThread | null {
    const file = path.join(stateDir(dir), "chat.json");
    try {
      const saved = readJson<{ version?: unknown; chat?: LegacyChat }>(file);
      if (!saved) return null;
      if (saved.version !== 1 || !saved.chat || saved.chat.items.length === 0) {
        fs.rmSync(file);
        return null;
      }
      const now = new Date().toISOString();
      const thread: SavedThread = {
        ...NEW_THREAD_SIDEBAR,
        ...saved.chat,
        id: randomUUID(),
        title: threadTitle(saved.chat.items),
        createdAt: now,
        updatedAt: now,
        archivedAt: null,
      };
      writeJson(threadFile(dir, thread.id), { version: VERSION, thread });
      fs.rmSync(file);
      return thread;
    } catch (error) {
      onError(error);
      return null;
    }
  }

  function readThreads(dir: string): SavedThread[] {
    const threads: SavedThread[] = [];
    const legacy = migrateLegacyChat(dir);
    let names: string[] = [];
    try {
      names = fs.readdirSync(threadsDir(dir)).filter((name) => name.endsWith(".json"));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") onError(error);
    }
    for (const name of names) {
      try {
        const saved = readJson<{ version?: unknown; thread?: SavedThread }>(path.join(threadsDir(dir), name));
        if (saved?.version === VERSION && saved.thread) threads.push(withSidebarFields(saved.thread));
      } catch (error) {
        onError(error);
      }
    }
    if (legacy && !threads.some((thread) => thread.id === legacy.id)) threads.push(legacy);
    return threads;
  }

  function saveProject(project: SavedProject) {
    try {
      prepare(project.dir);
      const file: ProjectFile = {
        version: VERSION,
        name: project.name,
        resolve: project.link,
        ...(project.icon ? { icon: project.icon } : {}),
        ...(project.collection ? { collection: { id: project.collection.id, name: project.collection.name } } : {}),
        ...(project.setup ? { setup: true } : {}),
        ...(project.autonomous === undefined ? {} : { autonomous: project.autonomous }),
        ...(project.drive ? { drive: project.drive } : {}),
      };
      writeJson(path.join(project.dir, PROJECT_FILE), file);
      const local: LocalJson = { ...project.local, ...(project.mirrored ? { mirror: { state: "mirrored" } } : {}) };
      writeJson(path.join(stateDir(project.dir), "local.json"), local);
      const entry = registry.projects.find((candidate) => candidate.id === project.id);
      if (entry && (entry.pinned === true) !== project.pinned) entry.pinned = project.pinned;
      if (entry && (entry.archived === true) !== project.archived) entry.archived = project.archived;
      saveRegistry();
    } catch (error) {
      onError(error);
    }
  }

  function uniqueDir(name: string): string {
    const base = folderName(name);
    const taken = (dir: string) => fs.existsSync(dir) || registry.projects.some((entry) => path.resolve(entry.dir) === path.resolve(dir));
    let dir = path.join(root, base);
    for (let n = 2; taken(dir); n += 1) dir = path.join(root, `${base} ${n}`);
    return dir;
  }

  return {
    load() {
      return registry.projects.map((entry) => {
        try {
          prepare(entry.dir);
        } catch (error) {
          onError(error);
        }
        return { project: readProject(entry), threads: readThreads(entry.dir) };
      });
    },
    create(fields) {
      const project: SavedProject = { icon: null, pinned: false, archived: false, ...fields, id: randomUUID(), dir: uniqueDir(fields.name ?? fields.link?.name ?? "Project") };
      registry = { ...registry, projects: [...registry.projects, { id: project.id, dir: project.dir }] };
      saveProject(project);
      return project;
    },
    adopt(dir) {
      const resolved = path.resolve(dir);
      if (!fs.existsSync(path.join(resolved, PROJECT_FILE))) {
        throw new Error(`${resolved} has no ${PROJECT_FILE}, so it isn't a Linker-Edit project folder.`);
      }
      if (registry.projects.some((entry) => path.resolve(entry.dir) === resolved)) {
        throw new Error(`${resolved} is already a project in Linker-Edit.`);
      }
      const id = randomUUID();
      prepare(resolved);
      registry = { ...registry, projects: [...registry.projects, { id, dir: resolved }] };
      saveRegistry();
      return { project: readProject({ id, dir: resolved }), threads: readThreads(resolved) };
    },
    saveProject,
    reload: ({ id, dir, pinned, archived }) => readProject({ id, dir, pinned, archived }),
    removeProject(id) {
      registry = { ...registry, projects: registry.projects.filter((entry) => entry.id !== id) };
      saveRegistry();
    },
    saveThread(project, thread) {
      try {
        prepare(project.dir);
        writeJson(threadFile(project.dir, thread.id), { version: VERSION, thread });
      } catch (error) {
        onError(error);
      }
    },
    deleteThread(project, threadId) {
      try {
        fs.rmSync(threadFile(project.dir, threadId), { force: true });
      } catch (error) {
        onError(error);
      }
    },
  };
}
