import { createHash, randomBytes } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import type { LinkerAccount } from "../shared/protocol.ts";

/**
 * Signing in with a Linker account (linker.cool), which is optional: nothing in Linker-Edit needs it yet.
 *
 * Linker's sign-in is Clerk's, and Clerk is an OAuth server, the same one Linker's AI connector uses. So this
 * is OAuth's authorization code flow with PKCE, for an app on the user's own computer (RFC 8252): the browser
 * signs in at Linker and comes back to this server on 127.0.0.1, which swaps the code for tokens.
 *
 * Clerk has to know the app asking. Linker serves a Client ID Metadata Document for Linker-Edit at
 * app.linker.cool/oauth/linker-edit.json, which Clerk reads (its client_id is that address). Until that is
 * live, or when this server's address isn't in it, Linker-Edit registers itself once (Dynamic Client
 * Registration) if Clerk allows it. `linkerClientId` in the config overrides both.
 */

export interface LinkerOptions {
  /** Where the sign-in is kept, readable only by this user. */
  file: string;
  /** Clerk's Frontend API, which is Linker's OAuth server. */
  authServer: string;
  /** Linker's dashboard and API. */
  appUrl: string;
  /** A client id to use instead of the metadata document or registering. */
  clientId?: string;
  /** This server's callback, on 127.0.0.1. */
  redirectUri: string;
  fetch?: typeof fetch;
  now?: () => number;
}

/** What Linker-Edit asks to see: who the person is, and to stay signed in. */
export const SCOPES = "openid profile email offline_access";

const SIGN_IN_TIMEOUT = 10 * 60_000;
const NETWORK_TIMEOUT = 15_000;
/** Refresh this long before the access token runs out. */
const EXPIRY_MARGIN = 60_000;

interface Client {
  id: string;
  secret?: string;
}

interface Registered extends Client {
  authServer: string;
  redirectUri: string;
}

interface Session {
  authServer: string;
  client: Client;
  accessToken: string;
  refreshToken: string | null;
  /** Milliseconds since 1970. */
  expiresAt: number;
  name: string;
  email: string | null;
  picture: string | null;
  plan: string | null;
}

interface Saved {
  /** A client Linker-Edit registered itself, kept after signing out so it isn't registered again. */
  registered?: Registered;
  session?: Session;
}

interface ServerMetadata {
  issuer: string;
  authorization_endpoint: string;
  token_endpoint: string;
  revocation_endpoint?: string;
  registration_endpoint?: string;
  userinfo_endpoint?: string;
  client_id_metadata_document_supported?: boolean;
}

interface TokenResponse {
  access_token?: string;
  refresh_token?: string;
  expires_in?: number;
  error?: string;
  error_description?: string;
}

/** A refusal from Linker's OAuth server, as opposed to not reaching it. */
class OAuthError extends Error {
  readonly code: string;
  constructor(code: string, message: string) {
    super(message);
    this.code = code;
  }
}

const base64url = (bytes: Buffer) => bytes.toString("base64url");

export function pkce(): { verifier: string; challenge: string } {
  const verifier = base64url(randomBytes(32));
  return { verifier, challenge: base64url(createHash("sha256").update(verifier).digest()) };
}

/** The metadata document Linker serves for Linker-Edit, which is also its client id. */
export function metadataDocumentUrl(appUrl: string): string {
  return `${appUrl.replace(/\/$/, "")}/oauth/linker-edit.json`;
}

export type LinkerService = ReturnType<typeof createLinker>;

export function createLinker(options: LinkerOptions) {
  const fetchFn = options.fetch ?? fetch;
  const now = options.now ?? Date.now;
  const authServer = options.authServer.replace(/\/$/, "");
  const appUrl = options.appUrl.replace(/\/$/, "");
  const listeners = new Set<() => void>();
  let saved = read();
  let pending: { state: string; verifier: string; client: Client; startedAt: number } | null = null;
  let problem: string | null = null;
  let metadata: ServerMetadata | null = null;

  function read(): Saved {
    try {
      return JSON.parse(fs.readFileSync(options.file, "utf8")) as Saved;
    } catch {
      return {};
    }
  }

  function write() {
    fs.mkdirSync(path.dirname(options.file), { recursive: true });
    fs.writeFileSync(options.file, JSON.stringify(saved, null, 2), { mode: 0o600 });
  }

  function changed() {
    for (const listener of listeners) listener();
  }

  function status(): LinkerAccount {
    const session = saved.session;
    if (session && session.authServer === authServer) {
      return { state: "signed_in", name: session.name, email: session.email, picture: session.picture, plan: session.plan, appUrl };
    }
    if (pending && now() - pending.startedAt < SIGN_IN_TIMEOUT) return { state: "signing_in" };
    return { state: "signed_out", problem };
  }

  async function call(url: string, init: RequestInit = {}): Promise<Response> {
    return fetchFn(url, { ...init, signal: AbortSignal.timeout(NETWORK_TIMEOUT) });
  }

  async function serverMetadata(): Promise<ServerMetadata> {
    if (metadata) return metadata;
    const res = await call(`${authServer}/.well-known/oauth-authorization-server`, { headers: { accept: "application/json" } });
    if (!res.ok) throw new Error(`Linker's sign-in answered ${res.status}.`);
    metadata = (await res.json()) as ServerMetadata;
    return metadata;
  }

  /** Whether Linker's metadata document for Linker-Edit is live, and the callbacks it lists when it is. */
  async function documentCallbacks(url: string): Promise<string[] | null> {
    try {
      const res = await call(url, { headers: { accept: "application/json" } });
      if (!res.ok) return null;
      const doc = (await res.json()) as { client_id?: unknown; redirect_uris?: unknown };
      if (doc.client_id !== url || !Array.isArray(doc.redirect_uris)) return null;
      return doc.redirect_uris.filter((uri): uri is string => typeof uri === "string");
    } catch {
      return null;
    }
  }

  async function register(meta: ServerMetadata, callbacks: string[] | null): Promise<Client> {
    const known = saved.registered;
    if (known && known.authServer === authServer && known.redirectUri === options.redirectUri) return known;
    const res = await call(meta.registration_endpoint ?? `${authServer}/oauth/register`, {
      method: "POST",
      headers: { "content-type": "application/json", accept: "application/json" },
      body: JSON.stringify({
        client_name: "Linker-Edit",
        client_uri: "https://github.com/FerretMedia/linker-edit",
        redirect_uris: [options.redirectUri],
        grant_types: ["authorization_code", "refresh_token"],
        response_types: ["code"],
        token_endpoint_auth_method: "none",
        scope: SCOPES,
      }),
    });
    const body = (await res.json().catch(() => ({}))) as { client_id?: string; client_secret?: string };
    if (!res.ok || !body.client_id) {
      // Linker knows Linker-Edit, only not at this address: the port was changed in the config.
      const port = callbacks?.map((uri) => URL.parse(uri)?.port).find(Boolean);
      throw new Error(
        port
          ? `Linker only lets Linker-Edit sign in on port ${port}. Take "port" out of linker-edit.config.json to sign in.`
          : "Linker doesn't accept Linker-Edit's sign-in yet. Try again after Linker's next release.",
      );
    }
    saved.registered = {
      id: body.client_id,
      ...(body.client_secret ? { secret: body.client_secret } : {}),
      authServer,
      redirectUri: options.redirectUri,
    };
    write();
    return saved.registered;
  }

  async function client(meta: ServerMetadata): Promise<Client> {
    if (options.clientId) return { id: options.clientId };
    const document = metadataDocumentUrl(appUrl);
    const callbacks = meta.client_id_metadata_document_supported ? await documentCallbacks(document) : null;
    if (callbacks?.includes(options.redirectUri)) return { id: document };
    return register(meta, callbacks);
  }

  function clientFields(client: Client): Record<string, string> {
    return { client_id: client.id, ...(client.secret ? { client_secret: client.secret } : {}) };
  }

  async function tokenRequest(meta: ServerMetadata, fields: Record<string, string>): Promise<TokenResponse> {
    const res = await call(meta.token_endpoint, {
      method: "POST",
      headers: { "content-type": "application/x-www-form-urlencoded", accept: "application/json" },
      body: new URLSearchParams(fields).toString(),
    });
    const body = (await res.json().catch(() => ({}))) as TokenResponse;
    if (!res.ok || !body.access_token) {
      throw new OAuthError(body.error ?? `http_${res.status}`, body.error_description ?? `Linker answered ${res.status}.`);
    }
    return body;
  }

  /** Who the token belongs to, and the plan of the organization their AI apps work on. */
  async function profile(meta: ServerMetadata, accessToken: string) {
    const auth = { authorization: `Bearer ${accessToken}`, accept: "application/json" };
    const res = await call(meta.userinfo_endpoint ?? `${authServer}/oauth/userinfo`, { headers: auth });
    if (!res.ok) throw new Error(`Linker didn't say who you are (${res.status}).`);
    const info = (await res.json()) as Record<string, unknown>;
    const text = (value: unknown) => (typeof value === "string" && value ? value : null);
    const fullName = [text(info.given_name), text(info.family_name)].filter(Boolean).join(" ");
    const email = text(info.email);
    // The plan is a nice-to-have: a 403 means the person switched the AI connector off in Linker.
    let plan: string | null = null;
    try {
      const account = await call(`${appUrl}/api/connector`, { headers: auth });
      if (account.ok) plan = text(((await account.json()) as { plan?: unknown }).plan);
    } catch {
      // Offline or Linker is busy; the name is enough.
    }
    return {
      name: text(info.name) ?? (fullName || null) ?? text(info.preferred_username) ?? email ?? "Linker account",
      email,
      picture: text(info.picture),
      plan,
    };
  }

  function signedOut(reason: string | null) {
    delete saved.session;
    write();
    problem = reason;
    changed();
  }

  /** A token that works for a while yet, refreshed when needed; null when signed out. */
  async function accessToken(): Promise<string | null> {
    const session = saved.session;
    if (!session || session.authServer !== authServer) return null;
    if (session.expiresAt - EXPIRY_MARGIN > now()) return session.accessToken;
    if (!session.refreshToken) {
      signedOut("Your Linker sign-in ran out. Sign in again.");
      return null;
    }
    const meta = await serverMetadata();
    try {
      const tokens = await tokenRequest(meta, {
        grant_type: "refresh_token",
        refresh_token: session.refreshToken,
        ...clientFields(session.client),
      });
      session.accessToken = tokens.access_token!;
      session.refreshToken = tokens.refresh_token ?? session.refreshToken;
      session.expiresAt = now() + (tokens.expires_in ?? 3600) * 1000;
      write();
      return session.accessToken;
    } catch (error) {
      // Only Linker saying no ends the sign-in; being offline doesn't.
      if (error instanceof OAuthError && error.code === "invalid_grant") signedOut("Your Linker sign-in ran out. Sign in again.");
      throw error;
    }
  }

  return {
    status,
    onStatus(listener: () => void) {
      listeners.add(listener);
    },

    /** Starts signing in; the browser goes to the address this returns. */
    async start(): Promise<string> {
      const meta = await serverMetadata();
      const chosen = await client(meta);
      const { verifier, challenge } = pkce();
      const state = base64url(randomBytes(16));
      pending = { state, verifier, client: chosen, startedAt: now() };
      problem = null;
      changed();
      const url = new URL(meta.authorization_endpoint);
      url.search = new URLSearchParams({
        response_type: "code",
        client_id: chosen.id,
        redirect_uri: options.redirectUri,
        scope: SCOPES,
        state,
        code_challenge: challenge,
        code_challenge_method: "S256",
      }).toString();
      return url.toString();
    },

    /** The browser's return from Linker. Answers with the name signed in as, or throws why not. */
    async finish(query: URLSearchParams): Promise<string> {
      const started = pending;
      if (!started || query.get("state") !== started.state || now() - started.startedAt > SIGN_IN_TIMEOUT) {
        throw new Error("This sign-in isn't the one Linker-Edit started, or it took too long. Start again from Linker-Edit.");
      }
      pending = null;
      try {
        const denied = query.get("error");
        if (denied) {
          throw new Error(denied === "access_denied" ? "Signing in was cancelled." : (query.get("error_description") ?? denied));
        }
        const meta = await serverMetadata();
        const iss = query.get("iss");
        if (iss && iss !== meta.issuer) throw new Error("The sign-in came back from somewhere other than Linker.");
        const code = query.get("code");
        if (!code) throw new Error("Linker sent no sign-in code.");
        const tokens = await tokenRequest(meta, {
          grant_type: "authorization_code",
          code,
          redirect_uri: options.redirectUri,
          code_verifier: started.verifier,
          ...clientFields(started.client),
        });
        const who = await profile(meta, tokens.access_token!);
        saved.session = {
          authServer,
          client: started.client,
          accessToken: tokens.access_token!,
          refreshToken: tokens.refresh_token ?? null,
          expiresAt: now() + (tokens.expires_in ?? 3600) * 1000,
          ...who,
        };
        write();
        problem = null;
        return who.name;
      } catch (error) {
        problem = error instanceof Error ? error.message : String(error);
        throw error;
      } finally {
        changed();
      }
    },

    cancel() {
      pending = null;
      changed();
    },

    async signOut(): Promise<void> {
      const session = saved.session;
      pending = null;
      signedOut(null);
      if (!session) return;
      // Best effort: the tokens are gone from this computer either way.
      try {
        const meta = await serverMetadata();
        if (!meta.revocation_endpoint) return;
        await call(meta.revocation_endpoint, {
          method: "POST",
          headers: { "content-type": "application/x-www-form-urlencoded" },
          body: new URLSearchParams({ token: session.refreshToken ?? session.accessToken, ...clientFields(session.client) }).toString(),
        });
      } catch {
        // Offline: Linker lets the token run out.
      }
    },

    accessToken,

    /** Brings the name and plan up to date, at start. Offline keeps what was there. */
    async refresh(): Promise<void> {
      try {
        const token = await accessToken();
        const session = saved.session;
        if (!token || !session) return;
        Object.assign(session, await profile(await serverMetadata(), token));
        write();
        changed();
      } catch {
        // Offline or Linker is busy: still signed in, with what was known.
      }
    },
  };
}
