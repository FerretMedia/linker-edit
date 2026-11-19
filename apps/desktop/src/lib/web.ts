/**
 * The page on someone's own web address: signing in with Linker for a web token, listing their computers, and
 * getting a session on one. Linker's side is FerretMedia/Linker's /edit/web routes; the host's is
 * /api/connect/session and /api/connect/native, the iPad app's way in. web.svelte.ts drives these.
 */
import type { LinkedComputer } from "#shared/protocol.ts";
import { helloAt, type Hello } from "./host.ts";

export const DEFAULT_LINKER_URL = "https://app.linker.cool";

type Store = Pick<Storage, "getItem" | "setItem" | "removeItem">;

const SIGN_IN_KEY = "linker-edit:web-sign-in";
const TOKEN_KEY = "linker-edit:web-token";
const SESSIONS_KEY = "linker-edit:web-sessions";
const LAST_KEY = "linker-edit:web-last";

/** A computer as Linker lists it: the parts the start screen uses. */
export type WebComputer = Pick<LinkedComputer, "id" | "name" | "url" | "tailnetUrl" | "online">;

/** Refused: by Linker, which wants a new sign-in, or by the host, which wants a new session. */
export class Unauthorized extends Error {
  readonly by: "linker" | "host";
  constructor(by: "linker" | "host", message: string) {
    super(message);
    this.by = by;
  }
}

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
    // A private window: the sign-in lasts only as long as the page.
  }
}

/** Linker's address: app.linker.cool, or what /linker-edit-web.json next to the page says. */
export async function linkerUrlOf(fetchFn: typeof fetch = fetch): Promise<string> {
  try {
    const res = await fetchFn("/linker-edit-web.json", { cache: "no-store" });
    // A missing file comes back as the page itself on Cloudflare, which isn't JSON.
    const body = (await res.json()) as { linkerUrl?: unknown };
    if (res.ok && typeof body.linkerUrl === "string" && /^https?:\/\/[^/]+/.test(body.linkerUrl)) return body.linkerUrl.replace(/\/+$/, "");
  } catch {
    // No file: the default.
  }
  return DEFAULT_LINKER_URL;
}

function base64url(bytes: Uint8Array): string {
  return btoa(String.fromCharCode(...bytes))
    .replace(/\+/g, "-")
    .replace(/\//g, "_")
    .replace(/=+$/, "");
}

const randomString = () => base64url(crypto.getRandomValues(new Uint8Array(32)));

/** PKCE's S256 challenge for a verifier. */
async function challengeOf(verifier: string): Promise<string> {
  return base64url(new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier))));
}

/** Where to send the browser to sign in; the state and verifier wait in this tab's sessionStorage for the return. */
export async function startSignIn(linkerUrl: string, origin: string, storage: Store): Promise<string> {
  const state = randomString();
  const verifier = randomString();
  write(storage, SIGN_IN_KEY, { state, verifier });
  const params = new URLSearchParams({ origin, state, code_challenge: await challengeOf(verifier), code_challenge_method: "S256" });
  return `${linkerUrl}/edit/web/authorize?${params}`;
}

export type SignInReturn = { code: string; state: string } | { error: string; state: string };

/** Linker's answer in the address's hash (#linker_code=…&state=…), or null when the hash isn't one. */
export function readReturn(hash: string): SignInReturn | null {
  const params = new URLSearchParams(hash.replace(/^#/, ""));
  const state = params.get("state") ?? "";
  const code = params.get("linker_code");
  if (code) return { code, state };
  const error = params.get("linker_error");
  return error ? { error, state } : null;
}

/** Swaps Linker's code for a web token and keeps it; throws with what to tell the user. */
export async function finishSignIn(
  linkerUrl: string,
  origin: string,
  answer: SignInReturn,
  session: Store,
  local: Store,
  fetchFn: typeof fetch = fetch,
): Promise<string> {
  const pending = read<{ state: string; verifier: string }>(session, SIGN_IN_KEY);
  write(session, SIGN_IN_KEY, null);
  // Only the tab that started a sign-in finishes it: a link someone else made has no verifier here.
  if (!pending || !answer.state || pending.state !== answer.state) throw new Error("That sign-in didn't start here. Sign in again.");
  if ("error" in answer) {
    throw new Error(answer.error === "access_denied" ? "This address wasn't allowed on Linker." : `Linker didn't sign you in (${answer.error}).`);
  }
  let res: Response;
  try {
    res = await fetchFn(`${linkerUrl}/api/edit/web/token`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({ code: answer.code, code_verifier: pending.verifier, origin }),
    });
  } catch {
    throw new Error("Couldn't reach Linker. Try again.");
  }
  const body = (await res.json().catch(() => ({}))) as { token?: unknown; expiresAt?: unknown };
  if (!res.ok || typeof body.token !== "string" || typeof body.expiresAt !== "string") {
    throw new Error("Linker didn't accept that sign-in. Sign in again.");
  }
  write(local, TOKEN_KEY, { token: body.token, expiresAt: body.expiresAt });
  return body.token;
}

/** The web token, until it runs out. */
export function savedToken(storage: Store, now = Date.now()): string | null {
  const saved = read<{ token?: unknown; expiresAt?: unknown }>(storage, TOKEN_KEY);
  if (typeof saved?.token === "string" && typeof saved.expiresAt === "string" && Date.parse(saved.expiresAt) > now) return saved.token;
  if (saved) write(storage, TOKEN_KEY, null);
  return null;
}

/** Signing out: the web token, every computer's session and the last computer go. */
export function forgetAll(storage: Store) {
  for (const key of [TOKEN_KEY, SESSIONS_KEY, LAST_KEY]) write(storage, key, null);
}

/** The user's computers, as Linker lists them for the web token. */
export async function listComputers(linkerUrl: string, token: string, fetchFn: typeof fetch = fetch): Promise<WebComputer[]> {
  let res: Response;
  try {
    res = await fetchFn(`${linkerUrl}/api/edit/computers`, { headers: { authorization: `Bearer ${token}`, accept: "application/json" } });
  } catch {
    throw new Error("Couldn't reach Linker. Check your connection and try again.");
  }
  if (res.status === 401) throw new Unauthorized("linker", "Your Linker sign-in ran out. Sign in again.");
  if (!res.ok) throw new Error(`Couldn't list your computers: Linker answered ${res.status}.`);
  const body = (await res.json().catch(() => ({}))) as { computers?: unknown };
  return (Array.isArray(body.computers) ? body.computers : []).flatMap((value: unknown) => {
    const computer = value as Partial<WebComputer> | null;
    if (typeof computer?.id !== "string" || typeof computer.name !== "string") return [];
    const address = (value: unknown) => (typeof value === "string" && value.startsWith("https://") ? value : null);
    return [
      {
        id: computer.id,
        name: computer.name,
        url: address(computer.url),
        tailnetUrl: address(computer.tailnetUrl),
        online: typeof computer.online === "boolean" ? computer.online : null,
      },
    ];
  });
}

export interface Reached {
  base: string;
  via: "tailscale" | "tunnel";
  hello: Hello;
}

/** Tailscale first, as it's direct and stays on the user's own network; else the tunnel; null when it's offline. */
export async function reach(computer: WebComputer, fetchFn: typeof fetch = fetch): Promise<Reached | null> {
  const tries = [
    { via: "tailscale" as const, address: computer.tailnetUrl, timeout: 2000 },
    // Linker knows when the tunnel is down; nothing to wait for then.
    { via: "tunnel" as const, address: computer.online === false ? null : computer.url, timeout: 5000 },
  ];
  for (const { via, address, timeout } of tries) {
    if (!address) continue;
    const base = new URL(address).origin;
    const hello = await helloAt(base, timeout, fetchFn);
    if (hello) return { base, via, hello };
  }
  return null;
}

/** "Browser on Windows": how the computer's Linker Connect settings list this device. */
export function deviceName(userAgent: string): string {
  const platforms: [RegExp, string][] = [
    [/iPad/, "iPad"],
    [/iPhone/, "iPhone"],
    [/Android/, "Android"],
    [/CrOS/, "ChromeOS"],
    [/Macintosh|Mac OS X/, "Mac"],
    [/Windows/, "Windows"],
    [/Linux/, "Linux"],
  ];
  return `Browser on ${platforms.find(([pattern]) => pattern.test(userAgent))?.[1] ?? "the web"}`;
}

type Sessions = Record<string, { session: string; expiresAt: string }>;

/** The session kept for a computer, until it runs out. */
export function savedSession(storage: Store, computerId: string, now = Date.now()): string | null {
  const saved = read<Sessions>(storage, SESSIONS_KEY)?.[computerId];
  return saved && Date.parse(saved.expiresAt) > now ? saved.session : null;
}

export function saveSession(storage: Store, computerId: string, session: { session: string; expiresAt: string } | null) {
  const sessions = { ...read<Sessions>(storage, SESSIONS_KEY) };
  if (session) sessions[computerId] = session;
  else delete sessions[computerId];
  write(storage, SESSIONS_KEY, sessions);
}

async function post(url: string, headers: Record<string, string>, body: unknown, fetchFn: typeof fetch) {
  const res = await fetchFn(url, { method: "POST", headers: { "content-type": "application/json", ...headers }, body: JSON.stringify(body) });
  return { res, body: (await res.json().catch(() => ({}))) as Record<string, unknown> };
}

/**
 * A new session on a computer, kept for next time: the web token buys a one-time code there (the computer asks
 * Linker who it belongs to), and the code a session. A 401 for the code means Linker no longer takes the token.
 */
export async function newSession(
  base: string,
  computerId: string,
  token: string,
  device: string,
  storage: Store,
  fetchFn: typeof fetch = fetch,
): Promise<string> {
  const admitted = await post(`${base}/api/connect/session`, { authorization: `Bearer ${token}` }, { name: device }, fetchFn);
  if (admitted.res.status === 401) throw new Unauthorized("linker", "Linker didn't accept your sign-in. Sign in again.");
  if (!admitted.res.ok || typeof admitted.body.code !== "string") {
    throw new Error(typeof admitted.body.error === "string" ? admitted.body.error : `The computer answered ${admitted.res.status}.`);
  }
  const redeemed = await post(`${base}/api/connect/native`, {}, { code: admitted.body.code }, fetchFn);
  const { session, expiresAt } = redeemed.body;
  if (!redeemed.res.ok || typeof session !== "string" || typeof expiresAt !== "string") {
    throw new Error(typeof redeemed.body.error === "string" ? redeemed.body.error : `The computer answered ${redeemed.res.status}.`);
  }
  saveSession(storage, computerId, { session, expiresAt });
  return session;
}

/** The computer that opens again next time. */
export const lastComputer = (storage: Store) => read<string>(storage, LAST_KEY);
export const rememberComputer = (storage: Store, computerId: string | null) => write(storage, LAST_KEY, computerId);
