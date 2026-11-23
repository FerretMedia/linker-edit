import { helloAt, Host, PROTOCOL, type Hello } from "./host.ts";
import { MachineConnection, machines } from "./machines.svelte.ts";
import { readCache } from "./machines.ts";
import {
  deviceName,
  finishSignIn,
  forgetAll,
  linkerUrlOf,
  listComputers,
  machineOf,
  newSession,
  reach,
  readReturn,
  saveSession,
  savedSession,
  savedToken,
  startSignIn,
  Unauthorized,
  type WebComputer,
} from "./web.ts";

type Phase =
  | { kind: "checking" }
  /** A computer serves this page: its server lists the machines. */
  | { kind: "host" }
  | { kind: "signed_out"; problem: string | null }
  /** On a web address, signed in: the app, with every computer Linker lists. */
  | { kind: "app" };

/** Shown once, with the first computer tried over Tailscale; Chrome asks about the local network for 100.x addresses. */
const NETWORK_KEY = "linker-edit:web-network-explained";
const TICKET_EARLY = 5 * 60_000;
const TICKET_RETRY = 60_000;
/** Linker is asked for the computers again on focus, at most this often. */
const RELIST_MS = 30_000;

/** Whether a computer serves this page; on a web address, the sign-in and a connection to each computer. */
class WebMode {
  phase = $state<Phase>({ kind: "checking" });
  /** Why Linker couldn't list the computers. */
  listProblem = $state<string | null>(null);
  /** The note about the browser asking for the local network, until it's dismissed. */
  networkNotice = $state(false);
  /** Each computer's hello, for its version. */
  #hellos = $state<Record<string, Hello>>({});
  #linkerUrl = "";
  #listedAt = 0;
  #opening = new Set<string>();
  #renewing = new Map<string, Promise<boolean>>();
  #tickets = new Map<string, ReturnType<typeof setTimeout>>();

  /** Says what to do when the page and a computer don't speak the same protocol. */
  hintFor(machineId: string): string | null {
    const hello = this.#hellos[machineId];
    if (!hello) return null;
    const theirs = hello.protocol ?? 0;
    if (theirs === PROTOCOL) return null;
    return theirs > PROTOCOL ? "Redeploy your web address in Cloudflare to update it." : `Update Linker-Edit on ${hello.name}.`;
  }

  async start() {
    if (await helloAt("", 1500)) {
      this.phase = { kind: "host" };
      void machines.startHere();
      return;
    }
    this.#linkerUrl = await linkerUrlOf();
    const answer = readReturn(location.hash);
    if (answer) {
      // The code is single use; it shouldn't stay in the address or the history.
      history.replaceState(history.state, "", location.pathname + location.search);
      try {
        await finishSignIn(this.#linkerUrl, location.origin, answer, sessionStorage, localStorage);
      } catch (error) {
        this.phase = { kind: "signed_out", problem: error instanceof Error ? error.message : String(error) };
        return;
      }
    }
    if (!savedToken(localStorage)) {
      this.phase = { kind: "signed_out", problem: null };
      return;
    }
    this.phase = { kind: "app" };
    window.addEventListener("focus", () => {
      if (this.phase.kind === "app" && Date.now() - this.#listedAt > RELIST_MS) void this.list();
    });
    await this.list();
  }

  async signIn() {
    location.assign(await startSignIn(this.#linkerUrl, location.origin, sessionStorage));
  }

  /** Lists the computers, and connects to each one not connected yet; the app shows meanwhile. */
  async list() {
    const token = savedToken(localStorage);
    if (!token) return this.signOut("Your Linker sign-in ran out. Sign in again.");
    this.#listedAt = Date.now();
    let computers: WebComputer[];
    try {
      computers = await listComputers(this.#linkerUrl, token);
    } catch (error) {
      if (error instanceof Unauthorized) return this.signOut(error.message);
      this.listProblem = error instanceof Error ? error.message : String(error);
      machines.loaded = true;
      return;
    }
    this.listProblem = null;
    if (computers.some((computer) => computer.tailnetUrl) && !this.#seen(NETWORK_KEY)) this.networkNotice = true;
    const cache = readCache(localStorage);
    machines.sync(
      computers.map(machineOf),
      (machine) => new MachineConnection(machine, new Host(), cache[machine.id] ?? null),
      false,
    );
    machines.loaded = true;
    for (const computer of computers) {
      const entry = machines.get(computer.id);
      if (entry && !entry.server.connected) void this.#open(entry, computer);
    }
  }

  dismissNetworkNotice() {
    this.networkNotice = false;
    try {
      localStorage.setItem(NETWORK_KEY, "1");
    } catch {
      // Shown again next time.
    }
  }

  #seen(key: string) {
    try {
      return localStorage.getItem(key) !== null;
    } catch {
      return false;
    }
  }

  /** Reaches a computer, then its kept session or a new one, a media ticket, and its socket. */
  async #open(entry: MachineConnection, computer: WebComputer) {
    if (this.#opening.has(computer.id)) return;
    this.#opening.add(computer.id);
    entry.unreachable = null;
    try {
      const reached = await reach(computer);
      if (machines.get(computer.id) !== entry) return;
      if (!reached) {
        entry.unreachable = "";
        return;
      }
      this.#hellos = { ...this.#hellos, [computer.id]: reached.hello };
      entry.machine = { ...entry.machine, version: reached.hello.version, via: reached.via === "tailscale" ? "tailscale" : "linker", online: true };
      const token = savedToken(localStorage);
      if (!token) return this.signOut("Your Linker sign-in ran out. Sign in again.");
      const session = savedSession(localStorage, computer.id) ?? (await newSession(reached.base, computer.id, token, deviceName(navigator.userAgent), localStorage));
      if (machines.get(computer.id) !== entry) return;
      entry.host.use({ base: reached.base, session });
      entry.host.renew = () => this.#renew(entry);
      entry.server.beforeReconnect = () => this.#refreshTicket(entry);
      // Before it connects, so its first media addresses carry the ticket.
      await this.#refreshTicket(entry);
      if (machines.get(computer.id) === entry) entry.server.connect();
    } catch (error) {
      if (error instanceof Unauthorized) return this.signOut(error.message);
      entry.unreachable = error instanceof TypeError ? `Couldn't reach ${computer.name}.` : error instanceof Error ? error.message : String(error);
    } finally {
      this.#opening.delete(computer.id);
    }
  }

  /** Forgets the web token, every session and what the computers had; with a problem, says why. */
  signOut(problem: string | null = null) {
    for (const timer of this.#tickets.values()) clearTimeout(timer);
    this.#tickets.clear();
    machines.clear();
    forgetAll(localStorage);
    this.networkNotice = false;
    this.#hellos = {};
    this.phase = { kind: "signed_out", problem };
  }

  /** A new session on a computer after it refused the old one; false when there's none to be had. */
  #renew(entry: MachineConnection): Promise<boolean> {
    const id = entry.id;
    const running = this.#renewing.get(id);
    if (running) return running;
    const renewing = (async () => {
      saveSession(localStorage, id, null);
      const token = savedToken(localStorage);
      if (!token) {
        this.signOut("Your Linker sign-in ran out. Sign in again.");
        return false;
      }
      try {
        entry.host.session = await newSession(entry.host.base, id, token, deviceName(navigator.userAgent), localStorage);
        return true;
      } catch (error) {
        if (error instanceof Unauthorized) this.signOut(error.message);
        else if (machines.get(id) === entry) {
          entry.server.disconnect();
          entry.unreachable = `${entry.name} signed this browser out, and didn't let it in again: ${error instanceof Error ? error.message : String(error)}`;
        }
        return false;
      }
    })().finally(() => this.#renewing.delete(id));
    this.#renewing.set(id, renewing);
    return renewing;
  }

  /** A computer's media ticket for `<video>` and `<img>` addresses, renewed 5 minutes before it runs out. */
  async #refreshTicket(entry: MachineConnection) {
    clearTimeout(this.#tickets.get(entry.id));
    if (!entry.host.web) return;
    let wait = TICKET_RETRY;
    try {
      const res = await entry.host.api("/api/connect/media-ticket", { method: "POST" });
      const body = (await res.json().catch(() => ({}))) as { ticket?: unknown; expiresAt?: unknown };
      if (res.ok && typeof body.ticket === "string" && typeof body.expiresAt === "string") {
        entry.host.ticket = body.ticket;
        wait = Math.max(TICKET_RETRY, Date.parse(body.expiresAt) - Date.now() - TICKET_EARLY);
      }
    } catch {
      // Offline for now: tried again below, and before the socket reconnects.
    }
    if (machines.get(entry.id) === entry) this.#tickets.set(entry.id, setTimeout(() => void this.#refreshTicket(entry), wait));
  }
}

export const web = new WebMode();
