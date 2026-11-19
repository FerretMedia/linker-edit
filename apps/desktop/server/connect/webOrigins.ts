import type { LinkerService } from "../linker.ts";

/**
 * The web addresses (https://linker-edit-web.jane.workers.dev) where the Linker account signed in here has put
 * Linker-Edit's UI and allowed it at Linker. Only pages there get CORS and the WebSocket from this computer
 * (server/http.ts); they still sign in like the iPad app. Linker keeps the list: Linker → Settings removes one.
 */

/** How long a list from Linker is used. */
const FRESH_FOR = 5 * 60_000;
/** An address not on the list asks Linker again at most this often, so one allowed a moment ago works at once. */
const MISS_EVERY = 30_000;
const NETWORK_TIMEOUT = 10_000;

export interface WebOriginsOptions {
  linker: Pick<LinkerService, "status" | "onStatus" | "accessToken" | "userId">;
  /** Linker's dashboard and API (app.linker.cool). */
  appUrl: string;
  fetch?: typeof fetch;
  now?: () => number;
}

export type WebOrigins = ReturnType<typeof createWebOrigins>;

export function createWebOrigins(options: WebOriginsOptions) {
  const fetchFn = options.fetch ?? fetch;
  const now = options.now ?? Date.now;
  const appUrl = options.appUrl.replace(/\/$/, "");
  let origins = new Set<string>();
  /** Whose list it is: another account signed in here doesn't get the last one's addresses. */
  let owner: string | null = null;
  let fetchedAt = -Infinity;
  let missAt = -Infinity;
  let loading: Promise<void> | null = null;

  const signedIn = () => options.linker.status().state === "signed_in";

  function forget() {
    origins = new Set();
    owner = null;
    fetchedAt = -Infinity;
    missAt = -Infinity;
  }
  options.linker.onStatus(() => {
    if (!signedIn()) forget();
  });

  async function load(): Promise<void> {
    const user = options.linker.userId();
    try {
      const token = await options.linker.accessToken();
      if (!token) {
        forget();
        return;
      }
      const res = await fetchFn(`${appUrl}/api/edit/web/origins`, {
        headers: { authorization: `Bearer ${token}`, accept: "application/json" },
        signal: AbortSignal.timeout(NETWORK_TIMEOUT),
      });
      if (!res.ok) throw new Error(`Linker answered ${res.status}.`);
      const body = (await res.json()) as { origins?: unknown };
      const list = Array.isArray(body.origins) ? body.origins.filter((origin): origin is string => typeof origin === "string") : [];
      origins = new Set(list);
      owner = user;
    } catch (error) {
      // Linker unreachable: the last list (if it's this account's) stands until the next try.
      if (owner !== user) origins = new Set();
      console.error(`Couldn't get your Linker-Edit web addresses from Linker: ${error instanceof Error ? error.message : String(error)}`);
    }
    fetchedAt = now();
  }

  function refresh(): Promise<void> {
    loading ??= load().finally(() => (loading = null));
    return loading;
  }

  return {
    /** Whether a page at origin may reach this computer; asks Linker when the list is old or doesn't have it. */
    async allows(origin: string | undefined): Promise<boolean> {
      if (!origin || origin === "null" || !signedIn()) return false;
      const stale = owner !== options.linker.userId() || now() - fetchedAt >= FRESH_FOR;
      if (stale) {
        missAt = now();
        await refresh();
      } else if (!origins.has(origin) && now() - missAt >= MISS_EVERY) {
        missAt = now();
        await refresh();
      }
      return signedIn() && origins.has(origin);
    },
  };
}
