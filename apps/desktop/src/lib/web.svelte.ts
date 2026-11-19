import { server } from "./connection.svelte.ts";
import { helloAt, host, PROTOCOL, type Hello } from "./host.ts";
import {
  deviceName,
  finishSignIn,
  forgetAll,
  lastComputer,
  linkerUrlOf,
  listComputers,
  newSession,
  reach,
  readReturn,
  rememberComputer,
  saveSession,
  savedSession,
  savedToken,
  startSignIn,
  Unauthorized,
  type Reached,
  type WebComputer,
} from "./web.ts";

export interface ComputerRow {
  computer: WebComputer;
  state: "checking" | "offline" | "ready" | "connecting";
  reached: Reached | null;
  problem: string | null;
}

type Phase =
  | { kind: "checking" }
  /** A computer serves this page: everything as it always was. */
  | { kind: "host" }
  | { kind: "signed_out"; problem: string | null }
  | { kind: "computers" }
  | { kind: "connected"; computer: WebComputer; hello: Hello };

/** Shown once, before the first try over Tailscale; Chrome asks about the local network for 100.x addresses. */
const NETWORK_KEY = "linker-edit:web-network-explained";
const TICKET_EARLY = 5 * 60_000;
const TICKET_RETRY = 60_000;

/** Whether a computer serves this page, and on a web address the start screen's sign-in and computer list. */
class WebMode {
  phase = $state<Phase>({ kind: "checking" });
  computers = $state<ComputerRow[] | null>(null);
  listProblem = $state<string | null>(null);
  /** The note about the browser asking for the local network, waiting for Continue. */
  askNetwork = $state(false);
  #linkerUrl = "";
  /** The computer being opened or open. */
  #open: WebComputer | null = null;
  #renewing: Promise<boolean> | null = null;
  #ticketTimer: ReturnType<typeof setTimeout> | undefined;

  /** Says what to do when the page and the computer don't speak the same protocol. */
  get versionHint(): string | null {
    if (this.phase.kind !== "connected") return null;
    const theirs = this.phase.hello.protocol ?? 0;
    if (theirs === PROTOCOL) return null;
    return theirs > PROTOCOL ? "Redeploy your web address in Cloudflare to update it." : `Update Linker-Edit on ${this.phase.hello.name}.`;
  }

  async start() {
    if (await helloAt("", 1500)) {
      this.phase = { kind: "host" };
      return;
    }
    host.renew = () => this.#renew();
    server.beforeReconnect = () => this.#refreshTicket();
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
    await this.list(lastComputer(localStorage));
  }

  async signIn() {
    location.assign(await startSignIn(this.#linkerUrl, location.origin, sessionStorage));
  }

  /** Lists the computers and looks for each one; `open` is the one to open once it answers. */
  async list(open: string | null = null, problem: string | null = null) {
    const token = savedToken(localStorage);
    if (!token) return this.signOut("Your Linker sign-in ran out. Sign in again.");
    this.phase = { kind: "computers" };
    this.listProblem = problem;
    this.computers = null;
    this.askNetwork = false;
    let computers: WebComputer[];
    try {
      computers = await listComputers(this.#linkerUrl, token);
    } catch (error) {
      if (error instanceof Unauthorized) return this.signOut(error.message);
      this.computers = null;
      this.listProblem = error instanceof Error ? error.message : String(error);
      return;
    }
    this.computers = computers.map((computer) => ({ computer, state: "checking", reached: null, problem: null }));
    if (computers.some((computer) => computer.tailnetUrl) && !this.#seen(NETWORK_KEY)) {
      this.askNetwork = true;
      return;
    }
    this.#look(open);
  }

  /** After the note about the local network: look for the computers. */
  continueToNetwork() {
    this.askNetwork = false;
    try {
      localStorage.setItem(NETWORK_KEY, "1");
    } catch {
      // Shown again next time.
    }
    this.#look(null);
  }

  #seen(key: string) {
    try {
      return localStorage.getItem(key) !== null;
    } catch {
      return false;
    }
  }

  #look(open: string | null) {
    for (const row of this.computers ?? []) {
      void reach(row.computer).then((reached) => {
        const current = this.#row(row.computer.id);
        if (!current || current.state !== "checking") return;
        current.reached = reached;
        current.state = reached ? "ready" : "offline";
        if (reached && current.computer.id === open && this.phase.kind === "computers") void this.pick(current.computer.id);
      });
    }
  }

  #row(id: string) {
    return this.computers?.find((row) => row.computer.id === id) ?? null;
  }

  /** Opens a computer: its kept session, or a new one, then a media ticket. */
  async pick(id: string) {
    const row = this.#row(id);
    const token = savedToken(localStorage);
    if (!row?.reached || !token) return;
    if (this.computers?.some((other) => other.state === "connecting")) return;
    row.state = "connecting";
    row.problem = null;
    const { base, hello } = row.reached;
    try {
      const session = savedSession(localStorage, id) ?? (await newSession(base, id, token, deviceName(navigator.userAgent), localStorage));
      host.use({ base, session });
      this.#open = row.computer;
      // Before the app shows, so its first media addresses carry the ticket.
      await this.#refreshTicket();
      if (this.#open !== row.computer) return;
      rememberComputer(localStorage, id);
      row.state = "ready";
      this.phase = { kind: "connected", computer: row.computer, hello };
    } catch (error) {
      this.#leave();
      if (error instanceof Unauthorized) return this.signOut(error.message);
      this.phase = { kind: "computers" };
      row.state = "ready";
      row.problem = error instanceof TypeError ? `Couldn't reach ${row.computer.name}.` : error instanceof Error ? error.message : String(error);
    }
  }

  /** Back to the computer list; it doesn't open by itself next time. */
  backToList() {
    this.#leave();
    rememberComputer(localStorage, null);
    void this.list();
  }

  /** Forgets the web token and every session; with a problem, says why. */
  signOut(problem: string | null = null) {
    this.#leave();
    forgetAll(localStorage);
    this.computers = null;
    this.askNetwork = false;
    this.phase = { kind: "signed_out", problem };
  }

  #leave() {
    this.#open = null;
    clearTimeout(this.#ticketTimer);
    server.disconnect();
    host.use(null);
  }

  /** A new session on the open computer after it refused the old one; false when there's none to be had. */
  #renew(): Promise<boolean> {
    const computer = this.#open;
    if (!computer) return Promise.resolve(false);
    const base = host.base;
    this.#renewing ??= (async () => {
      saveSession(localStorage, computer.id, null);
      const token = savedToken(localStorage);
      if (!token) {
        this.signOut("Your Linker sign-in ran out. Sign in again.");
        return false;
      }
      try {
        host.session = await newSession(base, computer.id, token, deviceName(navigator.userAgent), localStorage);
        return true;
      } catch (error) {
        if (error instanceof Unauthorized) this.signOut(error.message);
        else if (this.#open === computer) {
          this.#leave();
          void this.list(null, `${computer.name} signed this browser out, and didn't let it in again: ${error instanceof Error ? error.message : String(error)}`);
        }
        return false;
      }
    })().finally(() => (this.#renewing = null));
    return this.#renewing;
  }

  /** The media ticket for `<video>` and `<img>` addresses, renewed 5 minutes before it runs out. */
  async #refreshTicket() {
    clearTimeout(this.#ticketTimer);
    if (!host.web) return;
    let wait = TICKET_RETRY;
    try {
      const res = await host.api("/api/connect/media-ticket", { method: "POST" });
      const body = (await res.json().catch(() => ({}))) as { ticket?: unknown; expiresAt?: unknown };
      if (res.ok && typeof body.ticket === "string" && typeof body.expiresAt === "string") {
        host.ticket = body.ticket;
        wait = Math.max(TICKET_RETRY, Date.parse(body.expiresAt) - Date.now() - TICKET_EARLY);
      }
    } catch {
      // Offline for now: tried again below, and before the socket reconnects.
    }
    if (host.web) this.#ticketTimer = setTimeout(() => void this.#refreshTicket(), wait);
  }
}

export const web = new WebMode();
