import { createHash } from "node:crypto";
import { describe, expect, it, vi } from "vitest";
import {
  deviceName,
  finishSignIn,
  forgetAll,
  linkerUrlOf,
  listComputers,
  newSession,
  reach,
  readReturn,
  savedSession,
  savedToken,
  startSignIn,
  Unauthorized,
  type WebComputer,
} from "./web.ts";

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

function store() {
  const items = new Map<string, string>();
  return {
    items,
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    removeItem: (key: string) => void items.delete(key),
  };
}

const LINKER = "https://app.linker.cool";
const ORIGIN = "https://linker-edit-web.jane.workers.dev";
const hello = (name: string) => ({ app: "linker-edit", name, version: "2026.11.14", protocol: 1 });

describe("signing in with Linker", () => {
  it("sends the browser to Linker with an S256 challenge of a verifier kept in this tab", async () => {
    const session = store();
    const url = new URL(await startSignIn(LINKER, ORIGIN, session));
    expect(url.origin + url.pathname).toBe(`${LINKER}/edit/web/authorize`);
    expect(url.searchParams.get("origin")).toBe(ORIGIN);
    expect(url.searchParams.get("code_challenge_method")).toBe("S256");
    const pending = JSON.parse(session.getItem("linker-edit:web-sign-in")!) as { state: string; verifier: string };
    expect(url.searchParams.get("state")).toBe(pending.state);
    expect(pending.verifier).toMatch(/^[A-Za-z0-9_-]{43}$/);
    expect(url.searchParams.get("code_challenge")).toBe(createHash("sha256").update(pending.verifier).digest("base64url"));
    // A new state each time.
    await startSignIn(LINKER, ORIGIN, session);
    expect(JSON.parse(session.getItem("linker-edit:web-sign-in")!).state).not.toBe(pending.state);
  });

  it("reads Linker's answer from the hash, and nothing else", () => {
    expect(readReturn("#linker_code=abc&state=xyz")).toEqual({ code: "abc", state: "xyz" });
    expect(readReturn("#linker_error=access_denied&state=xyz")).toEqual({ error: "access_denied", state: "xyz" });
    expect(readReturn("#settings/general")).toBeNull();
    expect(readReturn("")).toBeNull();
  });

  it("swaps the code and verifier for a web token, and keeps it until it runs out", async () => {
    const session = store();
    const local = store();
    const url = new URL(await startSignIn(LINKER, ORIGIN, session));
    const { verifier } = JSON.parse(session.getItem("linker-edit:web-sign-in")!) as { verifier: string };
    const fetchFn = vi.fn(async () => json(200, { token: "lew_abc", expiresAt: "2026-12-19T00:00:00.000Z" }));

    await finishSignIn(LINKER, ORIGIN, { code: "c1", state: url.searchParams.get("state")! }, session, local, fetchFn);

    const [tokenUrl, init] = fetchFn.mock.calls[0] as unknown as [string, RequestInit];
    expect(tokenUrl).toBe(`${LINKER}/api/edit/web/token`);
    expect(JSON.parse(init.body as string)).toEqual({ code: "c1", code_verifier: verifier, origin: ORIGIN });
    // Single use: the verifier is gone.
    expect(session.items.size).toBe(0);
    expect(savedToken(local, Date.parse("2026-11-19T00:00:00Z"))).toBe("lew_abc");
    expect(savedToken(local, Date.parse("2026-12-20T00:00:00Z"))).toBeNull();
    expect(local.items.size).toBe(0);
  });

  it("refuses an answer whose state this tab didn't make, without asking Linker", async () => {
    const session = store();
    await startSignIn(LINKER, ORIGIN, session);
    const fetchFn = vi.fn(async () => json(200, {}));
    await expect(finishSignIn(LINKER, ORIGIN, { code: "c1", state: "forged" }, session, store(), fetchFn)).rejects.toThrow(/didn't start here/);
    // Nor in a tab that started nothing.
    await expect(finishSignIn(LINKER, ORIGIN, { code: "c1", state: "" }, store(), store(), fetchFn)).rejects.toThrow(/didn't start here/);
    expect(fetchFn).not.toHaveBeenCalled();
  });

  it("says when the address wasn't allowed, or Linker refused the code", async () => {
    const session = store();
    let state = new URL(await startSignIn(LINKER, ORIGIN, session)).searchParams.get("state")!;
    await expect(finishSignIn(LINKER, ORIGIN, { error: "access_denied", state }, session, store())).rejects.toThrow(/wasn't allowed/);

    state = new URL(await startSignIn(LINKER, ORIGIN, session)).searchParams.get("state")!;
    const local = store();
    const refused = async () => json(400, { error: "invalid_grant" });
    await expect(finishSignIn(LINKER, ORIGIN, { code: "c1", state }, session, local, refused)).rejects.toThrow(/didn't accept/);
    expect(local.items.size).toBe(0);
  });

  it("takes Linker's address from linker-edit-web.json, and the default without one", async () => {
    expect(await linkerUrlOf(async () => json(200, { linkerUrl: "https://staging.linker.cool/" }))).toBe("https://staging.linker.cool");
    // Cloudflare answers a missing file with the page.
    expect(await linkerUrlOf(async () => new Response("<!doctype html>"))).toBe(LINKER);
    expect(await linkerUrlOf(async () => json(200, { linkerUrl: "javascript:alert(1)" }))).toBe(LINKER);
  });

  it("forgets the token, the sessions and the last computer on sign out", () => {
    const local = store();
    local.setItem("linker-edit:web-token", "{}");
    local.setItem("linker-edit:web-sessions", "{}");
    local.setItem("linker-edit:web-last", '"pc"');
    local.setItem("linker-edit:drafts", "{}");
    forgetAll(local);
    expect([...local.items.keys()]).toEqual(["linker-edit:drafts"]);
  });
});

describe("the computer list", () => {
  it("lists the computers with the web token, and a 401 means signing in again", async () => {
    const fetchFn = vi.fn(async () =>
      json(200, {
        computers: [
          { id: "a", name: "Studio-PC", url: null, tailnetUrl: "https://studio.tail1.ts.net:8443", online: null, linkedAt: "" },
          { id: "b", name: "Laptop", url: "https://edit-b.linker.cool", online: true },
          { name: "no id" },
        ],
      }),
    );
    expect(await listComputers(LINKER, "lew_abc", fetchFn)).toEqual([
      { id: "a", name: "Studio-PC", url: null, tailnetUrl: "https://studio.tail1.ts.net:8443", online: null },
      { id: "b", name: "Laptop", url: "https://edit-b.linker.cool", tailnetUrl: null, online: true },
    ]);
    const [url, init] = fetchFn.mock.calls[0] as unknown as [string, RequestInit];
    expect(url).toBe(`${LINKER}/api/edit/computers`);
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer lew_abc");

    const refused = listComputers(LINKER, "lew_old", async () => json(401, { error: "unauthorized" }));
    await expect(refused).rejects.toBeInstanceOf(Unauthorized);
    await expect(refused).rejects.toMatchObject({ by: "linker" });
  });

  const computer = (fields: Partial<WebComputer>): WebComputer => ({
    id: "a",
    name: "Studio-PC",
    url: "https://edit-a.linker.cool",
    tailnetUrl: "https://studio.tail1.ts.net:8443",
    online: true,
    ...fields,
  });
  /** A fetch where only these origins answer as Linker-Edit. */
  const answering = (...origins: string[]) =>
    vi.fn(async (url: string | URL | Request) => {
      const origin = new URL(String(url)).origin;
      if (!origins.includes(origin)) throw new TypeError("Failed to fetch");
      return json(200, hello("Studio-PC"));
    });

  it("tries Tailscale first, then the tunnel, else it's offline", async () => {
    let fetchFn = answering("https://studio.tail1.ts.net:8443", "https://edit-a.linker.cool");
    expect(await reach(computer({}), fetchFn)).toMatchObject({ base: "https://studio.tail1.ts.net:8443", via: "tailscale" });
    expect(fetchFn).toHaveBeenCalledTimes(1);

    fetchFn = answering("https://edit-a.linker.cool");
    expect(await reach(computer({}), fetchFn)).toMatchObject({ base: "https://edit-a.linker.cool", via: "tunnel", hello: { name: "Studio-PC" } });
    expect(fetchFn.mock.calls.map(([url]) => String(url))).toEqual([
      "https://studio.tail1.ts.net:8443/api/hello",
      "https://edit-a.linker.cool/api/hello",
    ]);

    fetchFn = answering();
    expect(await reach(computer({}), fetchFn)).toBeNull();
    // Linked for Tailscale only, or with its tunnel down as Linker says: only Tailscale is tried.
    fetchFn = answering("https://edit-a.linker.cool");
    expect(await reach(computer({ url: null }), fetchFn)).toBeNull();
    expect(await reach(computer({ online: false }), fetchFn)).toBeNull();
    expect(fetchFn).toHaveBeenCalledTimes(2);
  });

  it("names this browser after its platform", () => {
    expect(deviceName("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Chrome/142.0")).toBe("Browser on Windows");
    expect(deviceName("Mozilla/5.0 (iPad; CPU OS 18_0 like Mac OS X) AppleWebKit/605.1.15")).toBe("Browser on iPad");
    expect(deviceName("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)")).toBe("Browser on Mac");
  });
});

describe("sessions", () => {
  const base = "https://edit-a.linker.cool";
  const computerFetch = (session: string) =>
    vi.fn(async (url: string | URL | Request) =>
      String(url).endsWith("/api/connect/session") ? json(200, { code: "one-time" }) : json(200, { session, expiresAt: "2026-12-19T00:00:00.000Z" }),
    );

  it("swaps the web token for a code and the code for a session, kept per computer", async () => {
    const local = store();
    const fetchFn = computerFetch("sa");
    expect(await newSession(base, "a", "lew_abc", "Browser on Windows", local, fetchFn)).toBe("sa");
    await newSession("https://studio.tail1.ts.net:8443", "b", "lew_abc", "Browser on Windows", local, computerFetch("sb"));

    const [[sessionUrl, admit], [nativeUrl, redeem]] = fetchFn.mock.calls as unknown as [[string, RequestInit], [string, RequestInit]];
    expect(sessionUrl).toBe(`${base}/api/connect/session`);
    expect(new Headers(admit.headers).get("authorization")).toBe("Bearer lew_abc");
    expect(JSON.parse(admit.body as string)).toEqual({ name: "Browser on Windows" });
    expect(nativeUrl).toBe(`${base}/api/connect/native`);
    // The session isn't asked for with the web token.
    expect(new Headers(redeem.headers).get("authorization")).toBeNull();
    expect(JSON.parse(redeem.body as string)).toEqual({ code: "one-time" });

    const now = Date.parse("2026-11-19T00:00:00Z");
    expect(savedSession(local, "a", now)).toBe("sa");
    expect(savedSession(local, "b", now)).toBe("sb");
    expect(savedSession(local, "c", now)).toBeNull();
    expect(savedSession(local, "a", Date.parse("2026-12-20T00:00:00Z"))).toBeNull();
  });

  it("a 401 for the code is Linker refusing the web token; other refusals say what the computer said", async () => {
    const local = store();
    const refused = newSession(base, "a", "lew_old", "Browser", local, async () => json(401, { error: "Linker didn't accept that sign-in." }));
    await expect(refused).rejects.toMatchObject({ by: "linker" });

    const otherAccount = newSession(base, "a", "lew_abc", "Browser", local, async () =>
      json(403, { error: "Studio-PC belongs to another Linker account." }),
    );
    await expect(otherAccount).rejects.toThrow("Studio-PC belongs to another Linker account.");
    await expect(otherAccount).rejects.not.toBeInstanceOf(Unauthorized);
    expect(savedSession(local, "a")).toBeNull();
  });
});
