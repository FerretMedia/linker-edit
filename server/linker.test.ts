import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { createHash } from "node:crypto";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { createLinker, metadataDocumentUrl, pkce, type LinkerOptions } from "./linker.ts";

const AUTH = "https://clerk.example";
const APP = "https://app.example";
const REDIRECT = "http://127.0.0.1:4317/api/linker/callback";
const DOCUMENT = metadataDocumentUrl(APP);

interface FakeOptions {
  cimd?: boolean;
  documentLists?: string[] | null;
  registration?: boolean;
  plan?: number;
  refresh?: "ok" | "invalid_grant" | "offline";
}

/** Clerk and Linker's API as far as signing in goes; records what was asked. */
function fakeLinker(fake: FakeOptions = {}) {
  const calls: { url: string; body: string }[] = [];
  let issued = 0;
  let verifier = "";
  let challenge = "";
  const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const fetchFn = (async (input: string | URL, init?: RequestInit) => {
    const url = String(input);
    const body = typeof init?.body === "string" ? init.body : "";
    calls.push({ url, body });
    if (url === `${AUTH}/.well-known/oauth-authorization-server`) {
      return json({
        issuer: AUTH,
        authorization_endpoint: `${AUTH}/oauth/authorize`,
        token_endpoint: `${AUTH}/oauth/token`,
        revocation_endpoint: `${AUTH}/oauth/token/revoke`,
        client_id_metadata_document_supported: fake.cimd ?? true,
      });
    }
    if (url === DOCUMENT) {
      return fake.documentLists === null ? new Response("Not found", { status: 404 }) : json({ client_id: DOCUMENT, redirect_uris: fake.documentLists ?? [REDIRECT] });
    }
    if (url === `${AUTH}/oauth/register`) {
      return fake.registration === false ? json({ error: "registration_disabled" }, 403) : json({ client_id: "registered-client" }, 201);
    }
    if (url === `${AUTH}/oauth/token`) {
      const form = new URLSearchParams(body);
      if (form.get("grant_type") === "authorization_code") {
        verifier = form.get("code_verifier") ?? "";
        if (createHash("sha256").update(verifier).digest("base64url") !== challenge) return json({ error: "invalid_grant" }, 400);
        return json({ access_token: `access-${++issued}`, refresh_token: "refresh-1", expires_in: 3600 });
      }
      if (fake.refresh === "offline") throw new TypeError("fetch failed");
      if (fake.refresh === "invalid_grant") return json({ error: "invalid_grant" }, 400);
      return json({ access_token: `access-${++issued}`, refresh_token: "refresh-2", expires_in: 3600 });
    }
    if (url === `${AUTH}/oauth/userinfo`) return json({ name: "Jane Doe", email: "jane@example.com", picture: "https://img.example/jane.png" });
    if (url === `${APP}/api/connector`) return json({ plan: "Max", paid: true }, fake.plan ?? 200);
    if (url === `${AUTH}/oauth/token/revoke`) return new Response(null, { status: 200 });
    return new Response("Not found", { status: 404 });
  }) as typeof fetch;
  return {
    calls,
    fetchFn,
    /** What Clerk keeps from the authorize request. */
    authorize(location: string) {
      const params = new URL(location).searchParams;
      challenge = params.get("code_challenge") ?? "";
      return params;
    },
  };
}

let dir: string;
let clock: number;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-"));
  clock = 1_000_000;
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

function linkerWith(fake: ReturnType<typeof fakeLinker>, extra: Partial<LinkerOptions> = {}) {
  return createLinker({
    file: path.join(dir, "linker-account.json"),
    authServer: AUTH,
    appUrl: APP,
    redirectUri: REDIRECT,
    fetch: fake.fetchFn,
    now: () => clock,
    ...extra,
  });
}

async function signIn(linker: ReturnType<typeof createLinker>, fake: ReturnType<typeof fakeLinker>) {
  const params = fake.authorize(await linker.start());
  return linker.finish(new URLSearchParams({ code: "the-code", state: params.get("state")!, iss: AUTH }));
}

describe("pkce", () => {
  it("makes an S256 challenge of the verifier", () => {
    const { verifier, challenge } = pkce();
    expect(verifier).toMatch(/^[A-Za-z0-9_-]{43}$/);
    expect(challenge).toBe(createHash("sha256").update(verifier).digest("base64url"));
  });
});

describe("signing in with Linker", () => {
  it("starts signed out, and isn't needed for anything", () => {
    const linker = linkerWith(fakeLinker());
    expect(linker.status()).toEqual({ state: "signed_out", problem: null });
  });

  it("signs in with Linker's metadata document as the client", async () => {
    const fake = fakeLinker();
    const linker = linkerWith(fake);
    const location = await linker.start();
    expect(linker.status()).toEqual({ state: "signing_in" });
    const params = fake.authorize(location);
    expect(location.startsWith(`${AUTH}/oauth/authorize?`)).toBe(true);
    expect(Object.fromEntries(params)).toMatchObject({
      response_type: "code",
      client_id: DOCUMENT,
      redirect_uri: REDIRECT,
      scope: "openid profile email offline_access",
      code_challenge_method: "S256",
    });

    const name = await linker.finish(new URLSearchParams({ code: "the-code", state: params.get("state")!, iss: AUTH }));
    expect(name).toBe("Jane Doe");
    expect(linker.status()).toEqual({
      state: "signed_in",
      name: "Jane Doe",
      email: "jane@example.com",
      picture: "https://img.example/jane.png",
      plan: "Max",
      appUrl: APP,
    });
    expect(fake.calls.some((call) => call.url.endsWith("/oauth/register"))).toBe(false);
  });

  it("keeps the sign-in for next time, readable by this user only", async () => {
    const fake = fakeLinker();
    await signIn(linkerWith(fake), fake);
    const file = path.join(dir, "linker-account.json");
    if (process.platform !== "win32") expect(fs.statSync(file).mode & 0o077).toBe(0);
    expect(linkerWith(fake).status()).toMatchObject({ state: "signed_in", name: "Jane Doe" });
  });

  it("registers itself once while the metadata document isn't live", async () => {
    const fake = fakeLinker({ documentLists: null });
    const linker = linkerWith(fake);
    expect(fake.authorize(await linker.start()).get("client_id")).toBe("registered-client");
    await linker.start();
    expect(fake.calls.filter((call) => call.url.endsWith("/oauth/register"))).toHaveLength(1);
    const registration = JSON.parse(fake.calls.find((call) => call.url.endsWith("/oauth/register"))!.body) as Record<string, unknown>;
    expect(registration).toMatchObject({ redirect_uris: [REDIRECT], token_endpoint_auth_method: "none" });
  });

  it("registers when the metadata document doesn't list this server's port", async () => {
    const fake = fakeLinker({ documentLists: ["http://127.0.0.1:9999/api/linker/callback"] });
    expect(fake.authorize(await linkerWith(fake).start()).get("client_id")).toBe("registered-client");
  });

  it("uses the configured client id over both", async () => {
    const fake = fakeLinker();
    expect(fake.authorize(await linkerWith(fake, { clientId: "my-client" }).start()).get("client_id")).toBe("my-client");
  });

  it("says so when Linker accepts neither", async () => {
    const linker = linkerWith(fakeLinker({ documentLists: null, registration: false }));
    await expect(linker.start()).rejects.toThrow("doesn't accept Linker-Edit's sign-in yet");
  });

  it("names the port Linker allows when the config changed it", async () => {
    const fake = fakeLinker({ documentLists: ["http://127.0.0.1:4318/api/linker/callback"], registration: false });
    await expect(linkerWith(fake).start()).rejects.toThrow("only lets Linker-Edit sign in on port 4318");
  });

  it("turns away a return it didn't start", async () => {
    const fake = fakeLinker();
    const linker = linkerWith(fake);
    fake.authorize(await linker.start());
    await expect(linker.finish(new URLSearchParams({ code: "x", state: "forged" }))).rejects.toThrow("isn't the one");
    // The real sign-in still works afterwards.
    expect(linker.status()).toEqual({ state: "signing_in" });
  });

  it("turns away a return from another issuer", async () => {
    const fake = fakeLinker();
    const linker = linkerWith(fake);
    const params = fake.authorize(await linker.start());
    await expect(linker.finish(new URLSearchParams({ code: "x", state: params.get("state")!, iss: "https://evil.example" }))).rejects.toThrow(
      "somewhere other than Linker",
    );
    expect(linker.status()).toEqual({ state: "signed_out", problem: "The sign-in came back from somewhere other than Linker." });
  });

  it("says when the person cancelled at Linker", async () => {
    const fake = fakeLinker();
    const linker = linkerWith(fake);
    const params = fake.authorize(await linker.start());
    await expect(linker.finish(new URLSearchParams({ error: "access_denied", state: params.get("state")! }))).rejects.toThrow("cancelled");
  });

  it("forgets a sign-in left waiting too long", async () => {
    const fake = fakeLinker();
    const linker = linkerWith(fake);
    const params = fake.authorize(await linker.start());
    clock += 11 * 60_000;
    expect(linker.status()).toEqual({ state: "signed_out", problem: null });
    await expect(linker.finish(new URLSearchParams({ code: "x", state: params.get("state")! }))).rejects.toThrow("took too long");
  });

  it("signs in without a plan when Linker's AI connector is switched off", async () => {
    const fake = fakeLinker({ plan: 403 });
    const linker = linkerWith(fake);
    await signIn(linker, fake);
    expect(linker.status()).toMatchObject({ state: "signed_in", plan: null });
  });
});

describe("staying signed in", () => {
  it("refreshes a token that ran out", async () => {
    const fake = fakeLinker();
    const linker = linkerWith(fake);
    await signIn(linker, fake);
    expect(await linker.accessToken()).toBe("access-1");
    clock += 2 * 3600_000;
    expect(await linker.accessToken()).toBe("access-2");
  });

  it("signs out when Linker refuses the refresh", async () => {
    const fake = fakeLinker({ refresh: "invalid_grant" });
    const linker = linkerWith(fake);
    await signIn(linker, fake);
    clock += 2 * 3600_000;
    await expect(linker.accessToken()).rejects.toThrow();
    expect(linker.status()).toEqual({ state: "signed_out", problem: "Your Linker sign-in ran out. Sign in again." });
  });

  it("stays signed in while offline", async () => {
    const fake = fakeLinker({ refresh: "offline" });
    const linker = linkerWith(fake);
    await signIn(linker, fake);
    clock += 2 * 3600_000;
    await linker.refresh();
    expect(linker.status()).toMatchObject({ state: "signed_in", name: "Jane Doe" });
  });

  it("signs out, revoking the token and forgetting it here", async () => {
    const fake = fakeLinker();
    const linker = linkerWith(fake);
    await signIn(linker, fake);
    await linker.signOut();
    expect(linker.status()).toEqual({ state: "signed_out", problem: null });
    expect(linkerWith(fake).status()).toEqual({ state: "signed_out", problem: null });
    const revoke = fake.calls.find((call) => call.url.endsWith("/revoke"));
    expect(new URLSearchParams(revoke?.body).get("token")).toBe("refresh-1");
  });
});
