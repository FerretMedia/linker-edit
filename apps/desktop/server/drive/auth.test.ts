import * as fs from "node:fs";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { FAKE_GOOGLE_CLIENT, FAKE_GOOGLE_USER, fakeGoogle } from "../../e2e/fake-google.ts";
import { createGoogleAuth, SCOPE, type GoogleAuth } from "./auth.ts";

let google: ReturnType<typeof fakeGoogle>;
let url: string;
let file: string;
let clock: number;
const auths: GoogleAuth[] = [];

beforeEach(async () => {
  google = fakeGoogle();
  await new Promise<void>((resolve) => google.server.listen(0, "127.0.0.1", resolve));
  url = `http://127.0.0.1:${(google.server.address() as AddressInfo).port}`;
  file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-google-")), "google-account.json");
  clock = Date.UTC(2026, 10, 26);
});

afterEach(() => {
  for (const auth of auths.splice(0)) auth.cancel();
  google.server.close();
});

function googleAuth(): GoogleAuth {
  const auth = createGoogleAuth({ client: FAKE_GOOGLE_CLIENT, file, url, now: () => clock });
  auths.push(auth);
  return auth;
}

/** What the browser does: Google's consent screen, then back to Linker-Edit's listener. */
async function consent(address: string): Promise<Response> {
  const google = await fetch(address, { redirect: "manual" });
  return fetch(google.headers.get("location")!);
}

async function signedIn(): Promise<GoogleAuth> {
  const auth = googleAuth();
  expect((await consent(await auth.start())).status).toBe(200);
  return auth;
}

describe("Google sign-in", () => {
  it("signs in through the browser with PKCE, and a restart stays signed in", async () => {
    const auth = googleAuth();
    const address = new URL(await auth.start());
    const ask = Object.fromEntries(address.searchParams);
    expect(ask).toMatchObject({ scope: SCOPE, access_type: "offline", prompt: "consent", code_challenge_method: "S256" });
    const back = new URL(ask.redirect_uri!);
    expect(back.hostname).toBe("127.0.0.1");
    expect(auth.status()).toEqual({ state: "signing_in" });

    // Anything else on this computer can reach the listener: without this sign-in's state it changes nothing.
    expect((await fetch(`${back}?code=stolen&state=guess`)).status).toBe(400);
    expect(auth.status()).toEqual({ state: "signing_in" });

    // A code asked for with another challenge, as by someone without the verifier, isn't exchanged.
    address.searchParams.set("code_challenge", "someone-elses-challenge");
    expect((await consent(address.toString())).status).toBe(400);
    expect(auth.status()).toMatchObject({ state: "signed_out", problem: expect.any(String) });

    expect((await consent(await auth.start())).status).toBe(200);
    expect(auth.status()).toEqual({ state: "signed_in", email: FAKE_GOOGLE_USER.emailAddress, name: FAKE_GOOGLE_USER.displayName });

    const restarted = googleAuth();
    expect(restarted.status()).toMatchObject({ state: "signed_in" });
    expect(restarted.accountId()).toBe(FAKE_GOOGLE_USER.permissionId);
  });

  it("says a sign-in cancelled at Google was cancelled", async () => {
    const auth = googleAuth();
    const back = new URL(await auth.start()).searchParams;
    const state = back.get("state")!;
    expect((await fetch(`${back.get("redirect_uri")}?error=access_denied&state=${state}`)).status).toBe(400);
    expect(auth.status()).toEqual({ state: "signed_out", problem: "Signing in was cancelled." });
  });

  it("refreshes a token that ran out or that Drive turned away, once for every caller", async () => {
    const auth = await signedIn();
    const first = await auth.token();
    const [a, b] = await Promise.all([auth.token(first), auth.token(first)]);
    expect(a).toBe(b);
    expect(a).not.toBe(first);
    expect(google.calls.filter((call) => call === "POST /token")).toHaveLength(2);

    clock += 2 * 3600_000;
    const later = await auth.token();
    expect(later).not.toBe(a);

    // The person took Linker-Edit's access back in their Google account.
    const { refreshToken } = JSON.parse(fs.readFileSync(file, "utf8")) as { refreshToken: string };
    await fetch(`${url}/revoke`, { method: "POST", body: new URLSearchParams({ token: refreshToken }) });
    await expect(auth.token(later)).rejects.toThrow();
    expect(auth.status()).toMatchObject({ state: "signed_out", problem: expect.any(String) });
    expect(fs.existsSync(file)).toBe(false);
  });

  it("signs out here and tells Google to forget the sign-in", async () => {
    const auth = await signedIn();
    await auth.signOut();
    expect(auth.status()).toEqual({ state: "signed_out", problem: null });
    expect(fs.existsSync(file)).toBe(false);
    expect(google.calls).toContain("POST /revoke");
    await expect(auth.token()).rejects.toThrow();
  });
});
