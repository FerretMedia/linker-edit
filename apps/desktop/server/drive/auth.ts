import { randomBytes } from "node:crypto";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as path from "node:path";
import type { GoogleDrive } from "../../shared/protocol.ts";
import { linkerPage } from "../http.ts";
import { pkce } from "../linker.ts";
import { createDriveClient, GOOGLE_API } from "./client.ts";

/**
 * Signing in to Google Drive, which the Drive sync works through (docs/brands-and-drive-plan.md).
 *
 * OAuth for an app on the user's own computer (RFC 8252): the authorization code flow with PKCE, and Google sends
 * the browser back to a listener on 127.0.0.1 at a port picked for each sign-in. The one scope is drive.file, so
 * Linker-Edit sees only the files it makes and the ones the user picks in Google's Picker, which allows no other
 * scope. That rules out OpenID's email scope too: who signed in comes from Drive's about.get.
 */

export const SCOPE = "https://www.googleapis.com/auth/drive.file";

/** A Google Cloud OAuth client of type Desktop app. Google says its secret isn't confidential. */
export interface GoogleClient {
  id: string;
  secret: string;
}

export interface GoogleAuthOptions {
  client: GoogleClient;
  /** Where the sign-in is kept, readable only by this user. */
  file: string;
  /** A stand-in for Google that serves its sign-in and Drive at their usual paths, for tests. */
  url?: string;
  fetch?: typeof fetch;
  now?: () => number;
}

interface Session {
  /** The client it was made with: another build's client can't refresh it. */
  clientId: string;
  accessToken: string;
  refreshToken: string;
  /** Milliseconds since 1970. */
  expiresAt: number;
  /** Drive's permissionId for the person, which stays when the email changes: a project's DriveLink.accountId. */
  accountId: string;
  email: string;
  name: string | null;
}

interface TokenResponse {
  access_token?: string;
  refresh_token?: string;
  expires_in?: number;
  scope?: string;
  error?: string;
  error_description?: string;
}

/** A refusal from Google's token endpoint, as opposed to not reaching it. */
class OAuthError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

const SIGN_IN_TIMEOUT = 10 * 60_000;
/** Refresh this long before the access token runs out. */
const EXPIRY_MARGIN = 60_000;
const RAN_OUT = "Your Google sign-in ran out or was taken back. Sign in again.";

function endpoints(url: string | undefined) {
  if (!url) {
    return {
      authorize: "https://accounts.google.com/o/oauth2/v2/auth",
      token: "https://oauth2.googleapis.com/token",
      revoke: "https://oauth2.googleapis.com/revoke",
      api: GOOGLE_API,
    };
  }
  const base = url.replace(/\/$/, "");
  return { authorize: `${base}/o/oauth2/v2/auth`, token: `${base}/token`, revoke: `${base}/revoke`, api: base };
}

function readSession(file: string): Session | null {
  try {
    const saved = JSON.parse(fs.readFileSync(file, "utf8")) as Partial<Session> | null;
    return typeof saved?.refreshToken === "string" && typeof saved.accountId === "string" ? (saved as Session) : null;
  } catch {
    return null;
  }
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

export type GoogleAuth = ReturnType<typeof createGoogleAuth>;

export function createGoogleAuth(options: GoogleAuthOptions) {
  const fetchFn = options.fetch ?? fetch;
  const now = options.now ?? Date.now;
  const google = endpoints(options.url);
  const listeners = new Set<() => void>();
  let saved = readSession(options.file);
  let pending: { state: string; verifier: string; redirectUri: string; listener: http.Server; timer: NodeJS.Timeout } | null = null;
  let problem: string | null = null;
  let refreshing: Promise<string> | null = null;

  const session = () => (saved?.clientId === options.client.id ? saved : null);

  function write() {
    fs.mkdirSync(path.dirname(options.file), { recursive: true });
    fs.writeFileSync(options.file, JSON.stringify(saved, null, 2), { mode: 0o600 });
  }

  function changed() {
    for (const listener of listeners) listener();
  }

  function stopWaiting() {
    if (!pending) return;
    clearTimeout(pending.timer);
    pending.listener.close();
    pending = null;
  }

  function signedOut(reason: string | null) {
    saved = null;
    fs.rmSync(options.file, { force: true });
    problem = reason;
    changed();
  }

  async function tokenRequest(fields: Record<string, string>): Promise<TokenResponse> {
    const res = await fetchFn(google.token, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded", accept: "application/json" },
      body: new URLSearchParams({ client_id: options.client.id, client_secret: options.client.secret, ...fields }).toString(),
    });
    const body = (await res.json().catch(() => ({}))) as TokenResponse;
    if (!res.ok || !body.access_token) {
      throw new OAuthError(body.error ?? `http_${res.status}`, body.error_description ?? `Google answered ${res.status}.`);
    }
    return body;
  }

  /** The browser's return from Google. Answers the email signed in as, or throws why not. */
  async function finish(query: URLSearchParams): Promise<string> {
    const started = pending;
    // Anything else on this computer can reach the listener too: only Google's answer to this sign-in ends it.
    if (!started || query.get("state") !== started.state) {
      throw new Error("This sign-in isn't the one Linker-Edit started. Start again from Linker-Edit's settings.");
    }
    stopWaiting();
    try {
      const denied = query.get("error");
      if (denied) throw new Error(denied === "access_denied" ? "Signing in was cancelled." : denied);
      const code = query.get("code");
      if (!code) throw new Error("Google sent no sign-in code.");
      const tokens = await tokenRequest({
        grant_type: "authorization_code",
        code,
        redirect_uri: started.redirectUri,
        code_verifier: started.verifier,
      });
      // Google's consent screen lets the person untick what an app asks for.
      if (tokens.scope && !tokens.scope.split(" ").includes(SCOPE)) {
        throw new Error("Linker-Edit needs to see the files it makes on your Drive. Sign in again and leave that ticked.");
      }
      if (!tokens.refresh_token) throw new Error("Google didn't let Linker-Edit stay signed in. Sign in again.");
      const user = await createDriveClient({ token: async () => tokens.access_token!, url: google.api, fetch: fetchFn }).about();
      saved = {
        clientId: options.client.id,
        accessToken: tokens.access_token!,
        refreshToken: tokens.refresh_token,
        expiresAt: now() + (tokens.expires_in ?? 3600) * 1000,
        accountId: user.permissionId,
        email: user.email,
        name: user.name,
      };
      write();
      problem = null;
      return user.email;
    } catch (error) {
      problem = errorText(error);
      throw error;
    } finally {
      changed();
    }
  }

  function answer(req: http.IncomingMessage, res: http.ServerResponse) {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    // The browser asks for its favicon too.
    if (url.pathname !== "/") {
      res.writeHead(404, { connection: "close" }).end();
      return;
    }
    const page = (status: number, title: string, text: string) =>
      res.writeHead(status, { "content-type": "text/html; charset=utf-8", "cache-control": "no-store", connection: "close" }).end(linkerPage(title, text));
    finish(url.searchParams).then(
      (email) => page(200, "Signed in to Google Drive", `As ${email}. Close this tab and go back to Linker-Edit.`),
      (error: unknown) => page(400, "Not signed in", errorText(error)),
    );
  }

  async function refresh(current: Session): Promise<string> {
    try {
      const tokens = await tokenRequest({ grant_type: "refresh_token", refresh_token: current.refreshToken });
      current.accessToken = tokens.access_token!;
      current.expiresAt = now() + (tokens.expires_in ?? 3600) * 1000;
      if (saved === current) write();
      return current.accessToken;
    } catch (error) {
      // Only Google saying no ends the sign-in; being offline doesn't.
      if (error instanceof OAuthError && error.code === "invalid_grant" && saved === current) signedOut(RAN_OUT);
      throw error;
    }
  }

  return {
    status(): Exclude<GoogleDrive, { state: "not_set_up" }> {
      const current = session();
      if (current) return { state: "signed_in", email: current.email, name: current.name };
      if (pending) return { state: "signing_in" };
      return { state: "signed_out", problem };
    },

    onStatus(listener: () => void) {
      listeners.add(listener);
    },

    /** Starts signing in; the browser goes to the address this returns, and comes back to a listener of its own. */
    async start(): Promise<string> {
      stopWaiting();
      const listener = http.createServer(answer);
      await new Promise<void>((resolve, reject) => {
        listener.once("error", reject);
        listener.listen(0, "127.0.0.1", resolve);
      });
      const redirectUri = `http://127.0.0.1:${(listener.address() as AddressInfo).port}`;
      const { verifier, challenge } = pkce();
      const state = randomBytes(16).toString("base64url");
      const timer = setTimeout(() => {
        stopWaiting();
        changed();
      }, SIGN_IN_TIMEOUT);
      timer.unref();
      pending = { state, verifier, redirectUri, listener, timer };
      problem = null;
      changed();
      const url = new URL(google.authorize);
      url.search = new URLSearchParams({
        client_id: options.client.id,
        redirect_uri: redirectUri,
        response_type: "code",
        scope: SCOPE,
        state,
        code_challenge: challenge,
        code_challenge_method: "S256",
        access_type: "offline",
        prompt: "consent",
      }).toString();
      return url.toString();
    },

    cancel() {
      stopWaiting();
      changed();
    },

    async signOut(): Promise<void> {
      const current = session();
      stopWaiting();
      signedOut(null);
      if (!current) return;
      // Best effort: the tokens are gone from this computer either way.
      await fetchFn(google.revoke, {
        method: "POST",
        headers: { "content-type": "application/x-www-form-urlencoded" },
        body: new URLSearchParams({ token: current.refreshToken }).toString(),
      }).catch(() => {});
    },

    /**
     * An access token for Drive, refreshed when it runs out, or when Drive turned this one away (rejected). Callers
     * at once share one refresh. Throws when signed out: DriveClientOptions.token.
     */
    async token(rejected?: string): Promise<string> {
      const current = session();
      if (!current) throw new Error("Sign in to Google Drive in Linker-Edit's settings.");
      const fresh = rejected === undefined ? current.expiresAt - EXPIRY_MARGIN > now() : current.accessToken !== rejected;
      if (fresh) return current.accessToken;
      refreshing ??= refresh(current).finally(() => (refreshing = null));
      return refreshing;
    },

    /** Drive's id for the person signed in here, to tell whose Drive a project is on; null when signed out. */
    accountId(): string | null {
      return session()?.accountId ?? null;
    },
  };
}
