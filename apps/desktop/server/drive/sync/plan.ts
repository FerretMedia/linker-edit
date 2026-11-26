import * as path from "node:path";
import { FOLDER_TYPE, type DriveChange } from "../client.ts";

/**
 * The Drive sync's planner: what to do so a project folder and its folder on Drive hold the same files again, from
 * what both sides last agreed on (the base), what this computer has now and what Drive has now. It does no I/O.
 *
 * Paths are relative to the project folder, with forward slashes.
 */

export interface RemoteFolder {
  name: string;
  parent: string;
}

export interface RemoteFile {
  name: string;
  parent: string;
  md5: string;
  size: number;
}

/** What Drive has inside a project's folder, by Drive id. Google's own documents have no bytes and aren't in it. */
export interface RemoteTree {
  folders: Record<string, RemoteFolder>;
  files: Record<string, RemoteFile>;
}

/** A file as this computer and Drive last agreed on it. */
export interface BaseFile {
  driveId: string;
  md5: string;
  size: number;
  mtimeMs: number;
}

export interface LocalFile {
  md5: string;
  size: number;
  mtimeMs: number;
}

/** The remote tree by path. A folder is at "" for the project's own folder. */
export interface RemoteView {
  files: Record<string, { driveId: string; md5: string; size: number }>;
  folders: Record<string, string>;
}

export type Action =
  /** driveId is the file on Drive to give new contents; null makes a new one. */
  | { kind: "upload"; path: string; driveId: string | null }
  | { kind: "download"; path: string; driveId: string }
  /** Follows a rename or move on Drive. */
  | { kind: "moveLocal"; from: string; to: string }
  /** Follows a rename or move on this computer. */
  | { kind: "moveRemote"; from: string; to: string; driveId: string }
  | { kind: "deleteLocal"; path: string }
  /** Into Drive's bin. */
  | { kind: "deleteRemote"; path: string; driveId: string }
  /** Changed on both sides: this computer's copy is renamed to copy, so Drive's can take its place. */
  | { kind: "setAside"; path: string; copy: string }
  /** Both sides have the same bytes: only the base changes. */
  | { kind: "agree"; path: string; file: BaseFile }
  | { kind: "forget"; path: string };

/** Never synced: each computer's own state and Linker-Edit's half-written files. .git: see the plan's open questions. */
const NOT_SYNCED = [".git/", ".linker-edit/threads/", ".linker-edit/local.json", ".linker-edit/drive-", "linker-edit.json.tmp"];

export const excluded = (file: string) => file === ".git" || NOT_SYNCED.some((prefix) => file.startsWith(prefix));

const join = (dir: string, name: string) => (dir ? `${dir}/${name}` : name);

/**
 * Applies changes from Drive's feed. The feed covers the whole account, so only what ends up inside the project's
 * folder is kept: a change's parent may be a folder that comes later in the same feed.
 */
export function applyChanges(tree: RemoteTree, changes: DriveChange[], root: string): RemoteTree {
  const folders = { ...tree.folders };
  const files = { ...tree.files };
  for (const change of changes) {
    delete folders[change.fileId];
    delete files[change.fileId];
    if (change.removed || change.file.trashed) continue;
    const { name, parents, mimeType, md5, size } = change.file;
    const parent = parents[0];
    if (!parent) continue;
    if (mimeType === FOLDER_TYPE) folders[change.fileId] = { name, parent };
    else if (md5 !== null) files[change.fileId] = { name, parent, md5, size: size ?? 0 };
  }
  const inside = new Map<string, boolean>([[root, true]]);
  const isInside = (id: string): boolean => {
    const known = inside.get(id);
    if (known !== undefined) return known;
    // Marked first, so a loop of folders ends.
    inside.set(id, false);
    const answer = id in folders && isInside(folders[id]!.parent);
    inside.set(id, answer);
    return answer;
  };
  const keep = <T extends { parent: string }>(entries: Record<string, T>) =>
    Object.fromEntries(Object.entries(entries).filter(([, entry]) => isInside(entry.parent)));
  return { folders: keep(folders), files: keep(files) };
}

function uniqueName(name: string, taken: Set<string>): string {
  const ext = path.posix.extname(name);
  const stem = name.slice(0, name.length - ext.length);
  let candidate = name;
  for (let n = 2; taken.has(candidate); n += 1) candidate = `${stem} (${n})${ext}`;
  taken.add(candidate);
  return candidate;
}

/**
 * The tree by path. Drive lets a folder hold two items with the same name, a disk doesn't: the one with the lowest
 * id keeps it and the others get " (2)" and on, the same on every computer.
 */
export function remotePaths(tree: RemoteTree, root: string): RemoteView {
  const children = new Map<string, { id: string; folder: boolean; name: string }[]>();
  const add = (id: string, parent: string, name: string, folder: boolean) => {
    const list = children.get(parent) ?? [];
    list.push({ id, folder, name });
    children.set(parent, list);
  };
  for (const [id, folder] of Object.entries(tree.folders)) add(id, folder.parent, folder.name, true);
  for (const [id, file] of Object.entries(tree.files)) add(id, file.parent, file.name, false);

  const view: RemoteView = { files: {}, folders: { "": root } };
  const queue: [string, string][] = [[root, ""]];
  for (let next = queue.shift(); next; next = queue.shift()) {
    const [folderId, dir] = next;
    const taken = new Set<string>();
    for (const child of (children.get(folderId) ?? []).sort((a, b) => (a.id < b.id ? -1 : 1))) {
      const at = join(dir, uniqueName(child.name, taken));
      if (child.folder) {
        view.folders[at] = child.id;
        queue.push([child.id, at]);
      } else {
        const { md5, size } = tree.files[child.id]!;
        view.files[at] = { driveId: child.id, md5, size };
      }
    }
  }
  return view;
}

/** Where this computer's copy of a file changed on both sides goes: "<name> (from <computer>)<ext>". */
export function conflictCopy(file: string, computer: string, taken: (file: string) => boolean): string {
  const dir = path.posix.dirname(file);
  const ext = path.posix.extname(file);
  const stem = path.posix.basename(file, ext);
  for (let n = 1; ; n += 1) {
    const copy = join(dir === "." ? "" : dir, `${stem} (from ${computer}${n > 1 ? ` ${n}` : ""})${ext}`);
    if (!taken(copy)) return copy;
  }
}

/** A side of one path against the base: what it has now. */
type Side = "none" | "new" | "same" | "changed" | "gone";

interface Case {
  path: string;
  base: BaseFile | undefined;
  local: LocalFile | undefined;
  remote: RemoteView["files"][string] | undefined;
  copy: () => string;
}

const agreeOn = ({ path: at, local, remote }: Case): Action => ({
  kind: "agree",
  path: at,
  file: { driveId: remote!.driveId, md5: remote!.md5, size: remote!.size, mtimeMs: local!.mtimeMs },
});

function bothChanged(c: Case): Action[] {
  if (c.local!.md5 === c.remote!.md5) return [agreeOn(c)];
  const copy = c.copy();
  return [
    { kind: "setAside", path: c.path, copy },
    { kind: "download", path: c.path, driveId: c.remote!.driveId },
    { kind: "upload", path: copy, driveId: null },
  ];
}

const download = ({ path: at, remote }: Case): Action[] => [{ kind: "download", path: at, driveId: remote!.driveId }];

/** What each pair of local and remote sides comes to. A change wins over a delete, so nothing is lost. */
const RULES: Partial<Record<`${Side}/${Side}`, (c: Case) => Action[]>> = {
  "new/none": ({ path: at }) => [{ kind: "upload", path: at, driveId: null }],
  "none/new": download,
  "new/new": bothChanged,
  "same/same": (c) => (c.local!.mtimeMs === c.base!.mtimeMs && c.remote!.driveId === c.base!.driveId ? [] : [agreeOn(c)]),
  "changed/same": ({ path: at, remote }) => [{ kind: "upload", path: at, driveId: remote!.driveId }],
  "same/changed": download,
  "changed/changed": bothChanged,
  "gone/same": ({ path: at, remote }) => [{ kind: "deleteRemote", path: at, driveId: remote!.driveId }],
  "same/gone": ({ path: at }) => [{ kind: "deleteLocal", path: at }],
  "gone/gone": ({ path: at }) => [{ kind: "forget", path: at }],
  "gone/changed": download,
  "changed/gone": ({ path: at }) => [{ kind: "upload", path: at, driveId: null }],
};

function side(base: BaseFile | undefined, now: { md5: string } | undefined): Side {
  if (!base) return now ? "new" : "none";
  if (!now) return "gone";
  return now.md5 === base.md5 ? "same" : "changed";
}

/** The order actions run in: a conflict's copy is out of the way before Drive's version comes down. */
const PHASE: Record<Action["kind"], number> = {
  setAside: 0,
  moveLocal: 1,
  moveRemote: 1,
  deleteLocal: 2,
  deleteRemote: 2,
  agree: 2,
  forget: 2,
  download: 3,
  upload: 4,
};

export interface SyncState {
  base: Record<string, BaseFile>;
  local: Record<string, LocalFile>;
  remote: RemoteView["files"];
  /** This computer's name, for conflict copies. */
  computer: string;
}

export function planSync({ base, local, remote, computer }: SyncState): Action[] {
  const paths = new Set([...Object.keys(base), ...Object.keys(local), ...Object.keys(remote)].filter((file) => !excluded(file)));
  const copies = new Set<string>();
  const taken = (file: string) => paths.has(file) || copies.has(file);
  const actions: Action[] = [];
  for (const at of [...paths].sort()) {
    const c: Case = {
      path: at,
      base: base[at],
      local: local[at],
      remote: remote[at],
      copy: () => {
        const copy = conflictCopy(at, computer, taken);
        copies.add(copy);
        return copy;
      },
    };
    actions.push(...(RULES[`${side(c.base, c.local)}/${side(c.base, c.remote)}`]?.(c) ?? []));
  }
  return pairMoves(actions, { base, local, remote, computer }).sort((a, b) => PHASE[a.kind] - PHASE[b.kind]);
}

/**
 * A file gone from one path and the same file new at another is a rename or a move: follow it, rather than send
 * the bytes again. On Drive the same file keeps its id; on this computer it keeps its md5.
 */
function pairMoves(actions: Action[], { base, local, remote }: SyncState): Action[] {
  const downloads = new Map<string, Extract<Action, { kind: "download" }>>();
  const uploads = new Map<string, Extract<Action, { kind: "upload" }>[]>();
  for (const action of actions) {
    if (action.kind === "download" && !base[action.path] && !local[action.path]) downloads.set(action.driveId, action);
    if (action.kind === "upload" && !base[action.path] && !remote[action.path] && local[action.path]) {
      const md5 = local[action.path]!.md5;
      uploads.set(md5, [...(uploads.get(md5) ?? []), action]);
    }
  }
  const replaced = new Set<Action>();
  const moves = new Map<Action, Action>();
  for (const action of actions) {
    if (action.kind === "deleteLocal") {
      const was = base[action.path]!;
      const moved = downloads.get(was.driveId);
      if (moved && remote[moved.path]!.md5 === was.md5 && !replaced.has(moved)) {
        replaced.add(moved);
        moves.set(action, { kind: "moveLocal", from: action.path, to: moved.path });
      }
    }
    if (action.kind === "deleteRemote") {
      const moved = uploads.get(base[action.path]!.md5)?.find((upload) => !replaced.has(upload));
      if (moved) {
        replaced.add(moved);
        moves.set(action, { kind: "moveRemote", from: action.path, to: moved.path, driveId: action.driveId });
      }
    }
  }
  return actions.filter((action) => !replaced.has(action)).map((action) => moves.get(action) ?? action);
}
