/**
 * The user's machines, as the page keeps them: which one is selected, what each last had, and how the list changes.
 * The pure part of machines.svelte.ts, which holds a connection to each machine.
 */
import type { Machine, ProjectSummary } from "#shared/protocol.ts";

type Store = Pick<Storage, "getItem" | "setItem" | "removeItem">;

const CACHE_KEY = "linker-edit:machines";
const SELECTION_KEY = "linker-edit:selection";

/** This computer, when its server is older than GET /api/machines or didn't answer it. */
export const THIS_MACHINE: Machine = { id: "this", name: "This computer", version: "", via: "this", online: true, base: "" };

/** The open thread: a thread id is only unique on its own machine. */
export interface Selection {
  machineId: string;
  threadId: string | null;
}

/** What a machine had the last time it was connected, so its projects still show while it's offline. */
export interface CachedMachine {
  name: string;
  projects: ProjectSummary[];
}

/** A key unique across machines, for a project or thread of one: "<machineId>:<id>". */
export const scopedKey = (machineId: string, id: string) => `${machineId}:${id}`;

function read<T>(storage: Store, key: string): T | null {
  try {
    return JSON.parse(storage.getItem(key) ?? "null") as T | null;
  } catch {
    return null;
  }
}

function write(storage: Store, key: string, value: unknown) {
  try {
    if (value === null) storage.removeItem(key);
    else storage.setItem(key, JSON.stringify(value));
  } catch {
    // A private window or full storage: only forgotten after a reload.
  }
}

export function readSelection(storage: Store): Selection | null {
  const saved = read<Partial<Selection>>(storage, SELECTION_KEY);
  if (typeof saved?.machineId !== "string") return null;
  return { machineId: saved.machineId, threadId: typeof saved.threadId === "string" ? saved.threadId : null };
}

export const saveSelection = (storage: Store, selection: Selection | null) => write(storage, SELECTION_KEY, selection);

export function readCache(storage: Store): Record<string, CachedMachine> {
  const saved = read<Record<string, Partial<CachedMachine>>>(storage, CACHE_KEY);
  const cache: Record<string, CachedMachine> = {};
  for (const [id, entry] of Object.entries(saved && typeof saved === "object" ? saved : {})) {
    if (typeof entry?.name === "string" && Array.isArray(entry.projects)) cache[id] = { name: entry.name, projects: entry.projects };
  }
  return cache;
}

export function cacheMachine(storage: Store, id: string, entry: CachedMachine) {
  write(storage, CACHE_KEY, { ...readCache(storage), [id]: entry });
}

/** Signing out of a web address: what its computers had isn't left behind. */
export function forgetMachines(storage: Store) {
  write(storage, CACHE_KEY, null);
  write(storage, SELECTION_KEY, null);
}

/** GET /api/machines' answer, this computer first; null when it isn't one, as from a server older than it. */
export function parseMachines(body: unknown): Machine[] | null {
  const list = (body as { machines?: unknown } | null)?.machines;
  if (!Array.isArray(list)) return null;
  const machines = list.flatMap((value: unknown): Machine[] => {
    const machine = value as Partial<Machine> | null;
    if (typeof machine?.id !== "string" || typeof machine.name !== "string" || typeof machine.base !== "string") return [];
    // Only this computer's own server, or a path on it: a page never talks to an address the list made up.
    if (machine.base !== "" && !/^\/machines\/[^/?#]+$/.test(machine.base)) return [];
    const via = machine.via === "linker" || machine.via === "tailscale" ? machine.via : "this";
    return [{ id: machine.id, name: machine.name, version: typeof machine.version === "string" ? machine.version : "", via, online: machine.online !== false, base: machine.base }];
  });
  const here = machines.find((machine) => machine.id === THIS_MACHINE.id) ?? THIS_MACHINE;
  return [here, ...machines.filter((machine) => machine !== here)];
}

/** How the page's machines change to match a new list: which to connect to, which to update, which to leave. */
export function diffMachines(current: readonly string[], next: readonly Machine[]): { add: Machine[]; update: Machine[]; remove: string[] } {
  const known = new Set(current);
  const listed = new Set(next.map((machine) => machine.id));
  return {
    add: next.filter((machine) => !known.has(machine.id)),
    update: next.filter((machine) => known.has(machine.id)),
    remove: current.filter((id) => !listed.has(id)),
  };
}

/**
 * The machine the main pane shows: the selected one once it has something to show, else the first that does, so a
 * machine still connecting doesn't hold up the others. Null when none has.
 */
export function activeMachine(selection: Selection | null, machines: readonly { id: string; ready: boolean }[]): string | null {
  const selected = machines.find((machine) => machine.id === selection?.machineId);
  if (selected?.ready) return selected.id;
  return machines.find((machine) => machine.ready)?.id ?? null;
}
