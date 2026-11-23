import { randomUUID } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import type { ApprovedEdit, CollectionIcon, CollectionKind, Lesson } from "../../shared/protocol.ts";

/** A library folder of a workspace or series. Empty until the library comes to collections. */
export type CollectionLibraryFolder = never;

/** A workspace or series as kept on disk. */
export interface SavedCollection {
  id: string;
  kind: CollectionKind;
  name: string;
  /** The workspace a series is in; null for a workspace. */
  parentId: string | null;
  icon: CollectionIcon;
  /** Its SKILL.md without the front matter: the style guide for the agent. */
  rules: string;
  library: CollectionLibraryFolder[];
  /** Licensing: its projects only use its own music. Not used yet. */
  musicOnlyFromHere: boolean;
  /** Its projects' agent makes the first cut without asking. */
  autonomous: boolean;
  /** What feedback taught the agent, oldest first. */
  lessons: Lesson[];
  /** Approved edits of its projects, newest first. */
  edits: ApprovedEdit[];
}

/** What a new collection starts with besides what create() is given. */
export const NEW_COLLECTION = { library: [], musicOnlyFromHere: false, autonomous: false, lessons: [], edits: [] } satisfies Partial<SavedCollection>;

export interface CollectionStore {
  /** Every collection that could be read, in the order they were made. */
  load(): SavedCollection[];
  create(fields: Pick<SavedCollection, "kind" | "name" | "parentId" | "icon" | "rules">): SavedCollection;
  save(collection: SavedCollection): void;
  /** Takes it out of the list and deletes its folder, rules included. */
  remove(id: string): void;
  /** Where its rules are: its SKILL.md, or a RULES.md not taken over yet. Null when it isn't in the list. */
  rulesFile(id: string): string | null;
  /** The agent's notes on example videos, one Markdown file each in its examples/ folder. */
  examples(id: string): { name: string; file: string }[];
  /** Writes or replaces the notes on one example; where they are, or null when the collection is gone. */
  saveExample(id: string, name: string, text: string): string | null;
  removeExample(id: string, name: string): void;
}

export const COLLECTION_FILE = "collection.json";
export const RULES_FILE = "SKILL.md";
/** What SKILL.md was called before; read until the first save writes SKILL.md in its place. */
const OLD_RULES_FILE = "RULES.md";
export const EXAMPLES_DIR = "examples";

/** "Reel 12 - launch.mp4" → "reel-12-launch": an example's notes are named after its video. */
export function exampleName(name: string): string {
  return collectionSlug(name.replace(/\.[a-z0-9]{2,5}$/i, ""));
}

// Starts with a key, so a file that begins with a line of dashes doesn't lose what is above the next one.
export const FRONT_MATTER = /^\uFEFF?---\r?\n(?=[\w-]+:)([\s\S]*?)\r?\n---[ \t]*(?:\r?\n|$)(?:[ \t]*\r?\n)*/;

/** SKILL.md without its front matter: what people edit in the app and agents read. */
export const skillBody = (text: string): string => text.replace(FRONT_MATTER, "");

/**
 * The front matter Linker-Edit writes, so the folder works as a skill in Claude Code and Codex too. A skill's name
 * is its folder's, as the Agent Skills format asks.
 */
function frontMatter(folder: string, { kind, name }: Pick<SavedCollection, "kind" | "name">): string {
  const about =
    kind === "workspace"
      ? `How videos for ${name} are edited. Use for every video of this brand.`
      : `How ${name} videos are edited. Use for every video in this series.`;
  return `name: ${folder}\ndescription: ${JSON.stringify(about)}`;
}

// Bump when a file changes shape; a file with another version is left alone rather than misread.
const VERSION = 1;

interface Registry {
  version: number;
  /** In the order they were made; folder is the name under collections/. */
  collections: { id: string; folder: string }[];
}

/** collection.json: everything but the rules, which have their own file for editing by hand. */
interface CollectionFile {
  version: number;
  id: string;
  kind: CollectionKind;
  name: string;
  parentId: string | null;
  icon: CollectionIcon;
  library: CollectionLibraryFolder[];
  musicOnlyFromHere: boolean;
  /** Missing in files from before the agent learned brands. */
  autonomous?: boolean;
  lessons?: Lesson[];
  edits?: ApprovedEdit[];
}

/** "Ferret Media" → "ferret-media", safe as a folder name everywhere. */
export function collectionSlug(name: string): string {
  const slug = name
    .normalize("NFKD")
    .replace(/[̀-ͯ]/g, "")
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 50)
    .replace(/-+$/, "");
  return slug || "collection";
}

function readJson<T>(file: string): T | null {
  try {
    return JSON.parse(fs.readFileSync(file, "utf8")) as T;
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw error;
  }
}

function readText(file: string): string | null {
  try {
    return fs.readFileSync(file, "utf8");
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw error;
  }
}

// Written whole and renamed, so a crash mid-write can't leave half a file.
function writeFile(file: string, text: string) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const temp = `${file}.tmp`;
  fs.writeFileSync(temp, text);
  fs.renameSync(temp, file);
}

const writeJson = (file: string, value: unknown) => writeFile(file, `${JSON.stringify(value, null, 2)}\n`);

/**
 * Workspaces and series, listed in <root>/collections.json, each in <root>/collections/<slug>/ with its
 * collection.json and SKILL.md. A file that can't be read is kept as it is, never overwritten.
 */
export function fileCollectionStore(options: { root: string; onError?: (error: unknown) => void }): CollectionStore {
  const { root } = options;
  const onError = options.onError ?? console.error;
  const registryFile = path.join(root, "collections.json");
  const foldersDir = path.join(root, "collections");
  const dirOf = (folder: string) => path.join(foldersDir, folder);

  let registry: Registry = readRegistry();
  // Collections whose collection.json couldn't be read: left as they are, never written over.
  const unreadable = new Set<string>();

  function readRegistry(): Registry {
    let problem: string | null = null;
    try {
      const saved = readJson<Registry>(registryFile);
      if (!saved) return { version: VERSION, collections: [] };
      if (saved.version === VERSION && Array.isArray(saved.collections)) return saved;
      problem = "is from another Linker-Edit version";
    } catch (error) {
      problem = `can't be read (${error instanceof Error ? error.message : String(error)})`;
    }
    // Kept, not overwritten: the list is rebuilt from the folders, and the old file stays to look at.
    const kept = `${registryFile}.bad-${Date.now()}`;
    try {
      fs.renameSync(registryFile, kept);
    } catch (error) {
      onError(error);
    }
    onError(new Error(`${registryFile} ${problem}. It was kept as ${kept}, and the list was rebuilt from the folders in ${foldersDir}.`));
    const rebuilt: Registry = { version: VERSION, collections: [] };
    for (const folder of collectionFolders()) {
      try {
        const file = readJson<CollectionFile>(path.join(dirOf(folder), COLLECTION_FILE));
        if (file?.version === VERSION && typeof file.id === "string") rebuilt.collections.push({ id: file.id, folder });
      } catch (error) {
        onError(error);
      }
    }
    save(rebuilt);
    return rebuilt;
  }

  function collectionFolders(): string[] {
    try {
      return fs
        .readdirSync(foldersDir, { withFileTypes: true })
        .filter((entry) => entry.isDirectory() && fs.existsSync(path.join(foldersDir, entry.name, COLLECTION_FILE)))
        .map((entry) => entry.name);
    } catch {
      return [];
    }
  }

  function save(next: Registry) {
    try {
      writeJson(registryFile, next);
    } catch (error) {
      onError(error);
    }
  }

  function read(id: string, folder: string): SavedCollection | null {
    const file = path.join(dirOf(folder), COLLECTION_FILE);
    try {
      const saved = readJson<CollectionFile>(file);
      if (!saved) throw new Error(`${file} is missing, so this workspace or series was left out.`);
      if (saved.version !== VERSION || saved.id !== id) {
        throw new Error(`${file} is from another Linker-Edit version or another list, so it was left out and left as it is.`);
      }
      const skill = readText(path.join(dirOf(folder), RULES_FILE));
      // All of a RULES.md is rules, whatever it starts with.
      const rules = skill === null ? (readText(path.join(dirOf(folder), OLD_RULES_FILE)) ?? "") : skillBody(skill);
      return {
        id,
        kind: saved.kind,
        name: saved.name,
        parentId: saved.parentId ?? null,
        icon: saved.icon,
        rules,
        library: Array.isArray(saved.library) ? saved.library : [],
        musicOnlyFromHere: saved.musicOnlyFromHere === true,
        autonomous: saved.autonomous === true,
        lessons: Array.isArray(saved.lessons) ? saved.lessons : [],
        edits: Array.isArray(saved.edits) ? saved.edits : [],
      };
    } catch (error) {
      unreadable.add(id);
      onError(error);
      return null;
    }
  }

  function write(folder: string, collection: SavedCollection) {
    const { rules, ...fields } = collection;
    const collectionFile = path.join(dirOf(folder), COLLECTION_FILE);
    const skillFile = path.join(dirOf(folder), RULES_FILE);
    const before = readJson<CollectionFile>(collectionFile);
    const skill = readText(skillFile);
    const found = FRONT_MATTER.exec(skill ?? "")?.[1];
    // Front matter someone wrote or changed by hand stays as it is; ours follows a rename.
    const own = before && found === frontMatter(folder, before) ? undefined : found;
    const file: CollectionFile = { version: VERSION, ...fields };
    writeJson(collectionFile, file);
    const empty = own === undefined && !rules.trim();
    writeFile(skillFile, empty ? "" : `---\n${own ?? frontMatter(folder, collection)}\n---\n\n${rules}`);
    // Its rules are in SKILL.md now. A RULES.md next to a SKILL.md was never read, so it stays.
    if (skill === null) fs.rmSync(path.join(dirOf(folder), OLD_RULES_FILE), { force: true });
  }

  function uniqueFolder(name: string): string {
    const base = collectionSlug(name);
    const taken = (folder: string) =>
      fs.existsSync(dirOf(folder)) || registry.collections.some((entry) => entry.folder.toLowerCase() === folder.toLowerCase());
    let folder = base;
    for (let n = 2; taken(folder); n += 1) folder = `${base}-${n}`;
    return folder;
  }

  const folderOf = (id: string) => registry.collections.find((entry) => entry.id === id)?.folder ?? null;

  return {
    load() {
      return registry.collections.flatMap(({ id, folder }) => read(id, folder) ?? []);
    },
    create(fields) {
      const collection: SavedCollection = { ...structuredClone(NEW_COLLECTION), ...fields, id: randomUUID() };
      const folder = uniqueFolder(fields.name);
      write(folder, collection);
      registry = { ...registry, collections: [...registry.collections, { id: collection.id, folder }] };
      save(registry);
      return collection;
    },
    save(collection) {
      const folder = folderOf(collection.id);
      if (!folder || unreadable.has(collection.id)) return;
      try {
        write(folder, collection);
      } catch (error) {
        onError(error);
      }
    },
    remove(id) {
      const folder = folderOf(id);
      if (!folder) return;
      registry = { ...registry, collections: registry.collections.filter((entry) => entry.id !== id) };
      save(registry);
      try {
        fs.rmSync(dirOf(folder), { recursive: true, force: true });
      } catch (error) {
        onError(error);
      }
    },
    rulesFile(id) {
      const folder = folderOf(id);
      if (!folder) return null;
      const file = path.join(dirOf(folder), RULES_FILE);
      const old = path.join(dirOf(folder), OLD_RULES_FILE);
      return !fs.existsSync(file) && fs.existsSync(old) ? old : file;
    },
    examples(id) {
      const folder = folderOf(id);
      if (!folder) return [];
      const dir = path.join(dirOf(folder), EXAMPLES_DIR);
      try {
        return fs
          .readdirSync(dir)
          .filter((name) => name.endsWith(".md"))
          .sort()
          .map((name) => ({ name: name.slice(0, -3), file: path.join(dir, name) }));
      } catch {
        return [];
      }
    },
    saveExample(id, name, text) {
      const folder = folderOf(id);
      if (!folder) return null;
      const file = path.join(dirOf(folder), EXAMPLES_DIR, `${exampleName(name)}.md`);
      writeFile(file, text);
      return file;
    },
    removeExample(id, name) {
      const folder = folderOf(id);
      if (!folder) return;
      try {
        fs.rmSync(path.join(dirOf(folder), EXAMPLES_DIR, `${exampleName(name)}.md`), { force: true });
      } catch (error) {
        onError(error);
      }
    },
  };
}
