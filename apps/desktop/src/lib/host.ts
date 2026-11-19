/**
 * Where the Linker-Edit server is. Served by a computer (host mode), it's this page's own origin, as always. On
 * someone's own web address (web mode), it's the edit computer picked on the start screen, reached cross-origin
 * with a Bearer session; see web.svelte.ts. Every server address in the UI goes through `host`.
 */

import { PROTOCOL_VERSION, type Hello as HostHello } from "#shared/protocol.ts";

/** Raised only on a breaking change between the page and the host; the host's /api/hello says its own. */
export const PROTOCOL = PROTOCOL_VERSION;

/** What GET /api/hello answers; hosts older than the web address leave `protocol` out. */
export type Hello = Omit<HostHello, "protocol"> & { protocol?: number };

export function isHello(value: unknown): value is Hello {
  const hello = value as Partial<Hello> | null;
  return !!hello && hello.app === "linker-edit" && typeof hello.name === "string" && typeof hello.version === "string";
}

/** A Linker-Edit's hello at an origin ("" for this page's own), or null when none answers in time. */
export async function helloAt(origin: string, timeoutMs: number, fetchFn: typeof fetch = fetch): Promise<Hello | null> {
  try {
    const res = await fetchFn(`${origin}/api/hello`, { signal: AbortSignal.timeout(timeoutMs), cache: "no-store" });
    const body: unknown = res.ok ? await res.json() : null;
    return isHello(body) ? body : null;
  } catch {
    return null;
  }
}

const LOCAL_NAMES = ["localhost", "127.0.0.1", "[::1]"];
/** `<video>`, `<img>` and `<audio>` can't send headers, so these take the media ticket in the address instead. */
const MEDIA = /^\/(preview|media|library)\/|^\/api\/timeline(\?|$)/;

interface WebHost {
  base: string;
  session: string;
  ticket: string | null;
}

export class Host {
  #web: WebHost | null = null;
  /** Web mode: called on a 401 from the host; true when it got a new session, so the request is sent once more. */
  renew: (() => Promise<boolean>) | null = null;

  /** Whether the page is on a web address, talking to a computer elsewhere. */
  get web(): boolean {
    return this.#web !== null;
  }

  /** The server's origin. */
  get base(): string {
    return this.#web?.base ?? location.origin;
  }

  /** Whether the server is this very computer: only there do Linker's sign-in and the folder dialogs work. */
  get local(): boolean {
    return LOCAL_NAMES.includes(new URL(this.base).hostname);
  }

  /** Web mode with this computer and session; null goes back to the page's own origin. */
  use(web: { base: string; session: string } | null) {
    this.#web = web ? { base: web.base, session: web.session, ticket: null } : null;
  }

  set session(session: string) {
    if (this.#web) this.#web.session = session;
  }

  set ticket(ticket: string | null) {
    if (this.#web) this.#web.ticket = ticket;
  }

  /** A server address for links and media; host mode keeps it relative, as before. */
  url(path: string): string {
    if (!this.#web) return path;
    const url = this.#web.base + path;
    if (!this.#web.ticket || !MEDIA.test(path)) return url;
    return `${url}${path.includes("?") ? "&" : "?"}t=${encodeURIComponent(this.#web.ticket)}`;
  }

  /** The WebSocket's address, and in web mode the session as a subprotocol: a browser can't set its headers. */
  ws(path: string): { url: string; protocols?: string[] } {
    const url = this.base.replace(/^http/, "ws") + path;
    return this.#web ? { url, protocols: ["linker-edit", `bearer.${this.#web.session}`] } : { url };
  }

  /** fetch on the server, with the session in web mode. */
  async api(path: string, init: RequestInit = {}): Promise<Response> {
    if (!this.#web) return fetch(path, init);
    const send = () => {
      const headers = new Headers(init.headers);
      headers.set("authorization", `Bearer ${this.#web?.session ?? ""}`);
      return fetch((this.#web?.base ?? "") + path, { ...init, headers });
    };
    const res = await send();
    if (res.status !== 401 || !this.renew || !(await this.renew())) return res;
    return send();
  }
}

export const host = new Host();
