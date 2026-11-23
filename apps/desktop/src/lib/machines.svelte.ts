import type { Machine, ProjectSummary } from "#shared/protocol.ts";
import { ServerConnection } from "./connection.svelte.ts";
import { Host } from "./host.ts";
import {
  activeMachine,
  cacheMachine,
  diffMachines,
  forgetMachines,
  parseMachines,
  readCache,
  readSelection,
  saveSelection,
  THIS_MACHINE,
  type CachedMachine,
  type Selection,
} from "./machines.ts";

export type MachineStatus = "connecting" | "connected" | "offline";

const REFRESH_MS = 60_000;

/** One of the user's computers, with its own connection; like a T3 Code environment. */
export class MachineConnection {
  machine = $state<Machine>(THIS_MACHINE);
  readonly host: Host;
  readonly server: ServerConnection;
  /** Web mode: it couldn't be reached, or let this browser in; says why when there's a reason to give. */
  unreachable = $state<string | null>(null);
  /** What it had the last time it was connected. */
  cached = $state<CachedMachine | null>(null);

  constructor(machine: Machine, host: Host, cached: CachedMachine | null, options: { reloads?: boolean } = {}) {
    this.machine = machine;
    this.host = host;
    this.cached = cached;
    this.server = new ServerConnection(host, options);
  }

  get id(): string {
    return this.machine.id;
  }

  /** The computer's own name once it's connected, as it may have been renamed since the list was made. */
  get name(): string {
    return this.server.state?.computer ?? this.cached?.name ?? this.machine.name;
  }

  get status(): MachineStatus {
    if (this.server.connected) return "connected";
    if (this.unreachable !== null || !this.machine.online || this.server.state) return "offline";
    return "connecting";
  }

  /** Live while connected; otherwise what it had last, shown read-only. */
  get projects(): ProjectSummary[] {
    return this.server.state?.projects ?? this.cached?.projects ?? [];
  }
}

/** Every machine the page shows, each connected at once, and which thread on which machine is open. */
class Machines {
  list = $state<MachineConnection[]>([]);
  /** False until the first list is in: the page waits for it rather than flash "no computers". */
  loaded = $state(false);
  selection = $state<Selection | null>(readSelection(localStorage));
  #refreshTimer: ReturnType<typeof setInterval> | undefined;
  /** Stops keeping each machine's projects. */
  #stops = new Map<string, () => void>();

  get(id: string): MachineConnection | null {
    return this.list.find((entry) => entry.id === id) ?? null;
  }

  /** The machine the main pane shows; see activeMachine. */
  get active(): MachineConnection | null {
    const id = activeMachine(
      this.selection,
      this.list.map((entry) => ({ id: entry.id, ready: entry.server.state !== null })),
    );
    return id === null ? null : this.get(id);
  }

  /** Opens a thread on a machine; null leaves the thread to that machine's server, as after adding a project. */
  select(machineId: string, threadId: string | null) {
    this.selection = { machineId, threadId };
    saveSelection(localStorage, this.selection);
    const entry = this.get(machineId);
    if (threadId && entry?.server.connected) entry.server.send({ type: "select_thread", threadId });
  }

  /** The thread the machine's server moved this page to, as after a new thread or an archived one. */
  follow(machineId: string, threadId: string | null) {
    if (this.selection?.machineId === machineId && this.selection.threadId === threadId) return;
    if (this.selection && this.selection.machineId !== machineId) return;
    this.selection = { machineId, threadId };
    saveSelection(localStorage, this.selection);
  }

  /** Connects a new machine, keeping its projects for when it's offline later. */
  add(entry: MachineConnection) {
    entry.server.onConnected = (state) => {
      // The server picked a thread for this new connection; the remembered one goes first, only on the open machine
      // (a background machine's pick isn't shown, so it isn't marked read either).
      const want = this.selection?.machineId === entry.id ? this.selection.threadId : null;
      if (want && state.threadId !== want) entry.server.send({ type: "select_thread", threadId: want });
    };
    this.list = [...this.list, entry];
    this.#stops.set(
      entry.id,
      $effect.root(() => {
        $effect(() => {
          const state = entry.server.state;
          if (state) cacheMachine(localStorage, entry.id, { name: state.computer, projects: $state.snapshot(state.projects) as ProjectSummary[] });
        });
      }),
    );
  }

  remove(id: string) {
    this.#stops.get(id)?.();
    this.#stops.delete(id);
    this.get(id)?.server.disconnect();
    this.list = this.list.filter((entry) => entry.id !== id);
  }

  /** Leaves every machine, as when signing out of a web address. */
  clear() {
    clearInterval(this.#refreshTimer);
    for (const entry of this.list) this.remove(entry.id);
    this.loaded = false;
    this.selection = null;
    forgetMachines(localStorage);
  }

  /**
   * Host mode: the machines this computer's server lists, this computer first, each reached through it. A server
   * older than the list is just this computer. Looked at again on focus and every minute.
   */
  async startHere() {
    await this.#loadHere();
    this.loaded = true;
    this.#refreshTimer = setInterval(() => void this.#loadHere(), REFRESH_MS);
    window.addEventListener("focus", () => void this.#loadHere());
  }

  async #loadHere() {
    let machines: Machine[] | null = null;
    try {
      const res = await fetch("/api/machines", { cache: "no-store" });
      machines = res.ok ? parseMachines(await res.json()) : null;
    } catch {
      // An older server, or this one restarting: this computer alone for now.
    }
    // Without a list, the machines already shown stay; only the first time is it this computer alone.
    if (!machines && this.list.length) return;
    this.sync(machines ?? [THIS_MACHINE], (machine) => {
      const local = machine.base === "";
      return new MachineConnection(machine, new Host(machine.base), readCache(localStorage)[machine.id] ?? null, { reloads: local });
    });
  }

  /** Matches the machines to a new list: new ones connect, gone ones close, the rest get the new details. */
  sync(next: Machine[], make: (machine: Machine) => MachineConnection, connect = true) {
    const { add, update, remove } = diffMachines(
      this.list.map((entry) => entry.id),
      next,
    );
    for (const id of remove) this.remove(id);
    for (const machine of update) {
      const entry = this.get(machine.id)!;
      entry.machine = machine;
      // Back online: connect again now rather than at the next retry.
      if (connect && machine.online) entry.server.connect();
    }
    for (const machine of add) {
      const entry = make(machine);
      this.add(entry);
      if (connect && machine.online) entry.server.connect();
    }
    // In the listed order, this computer first.
    this.list = next.map((machine) => this.get(machine.id)!).filter(Boolean);
  }
}

export const machines = new Machines();
