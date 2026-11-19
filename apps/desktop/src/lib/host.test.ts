import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { helloAt, Host } from "./host.ts";

const json = (status: number, body: unknown) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });

describe("host", () => {
  let fetchMock: ReturnType<typeof vi.fn>;
  beforeEach(() => {
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4317" });
    fetchMock = vi.fn(async () => json(200, {}));
    vi.stubGlobal("fetch", fetchMock);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("is this page's own origin when a computer serves it, as before", async () => {
    const host = new Host();
    expect(host.web).toBe(false);
    expect(host.local).toBe(true);
    expect(host.url("/media/a.mov")).toBe("/media/a.mov");
    expect(host.ws("/ws?meta=patch")).toEqual({ url: "ws://127.0.0.1:4317/ws?meta=patch" });
    await host.api("/api/timeline?id=1");
    expect(fetchMock).toHaveBeenCalledWith("/api/timeline?id=1", {});
  });

  it("on a web address, is the computer picked, with the session in a header and a subprotocol, never in a URL", async () => {
    const host = new Host();
    host.use({ base: "https://pc.tail1.ts.net:8443", session: "s1" });
    expect(host.local).toBe(false);
    expect(host.ws("/ws?meta=patch")).toEqual({ url: "wss://pc.tail1.ts.net:8443/ws?meta=patch", protocols: ["linker-edit", "bearer.s1"] });
    // No ticket yet: media addresses go out bare.
    expect(host.url("/media/a.mov")).toBe("https://pc.tail1.ts.net:8443/media/a.mov");

    host.ticket = "t/1";
    expect(host.url("/media/a.mov")).toBe("https://pc.tail1.ts.net:8443/media/a.mov?t=t%2F1");
    expect(host.url("/preview/p.mp4")).toBe("https://pc.tail1.ts.net:8443/preview/p.mp4?t=t%2F1");
    expect(host.url("/library/f/Tone.wav")).toBe("https://pc.tail1.ts.net:8443/library/f/Tone.wav?t=t%2F1");
    expect(host.url("/api/timeline?id=1")).toBe("https://pc.tail1.ts.net:8443/api/timeline?id=1&t=t%2F1");
    expect(host.url("/api/linker/sign-in")).toBe("https://pc.tail1.ts.net:8443/api/linker/sign-in");

    await host.api("/api/timeline", { headers: { accept: "application/json" } });
    const [url, init] = fetchMock.mock.calls[0] as [string, RequestInit];
    expect(url).toBe("https://pc.tail1.ts.net:8443/api/timeline");
    expect(new Headers(init.headers).get("authorization")).toBe("Bearer s1");
    expect(new Headers(init.headers).get("accept")).toBe("application/json");
  });

  it("gets a new session on a 401 from the computer and sends the request once more", async () => {
    const host = new Host();
    host.use({ base: "https://edit-1.linker.cool", session: "old" });
    host.renew = vi.fn(async () => {
      host.session = "new";
      return true;
    });
    fetchMock.mockResolvedValueOnce(json(401, { error: "Signed out." })).mockResolvedValueOnce(json(200, { ok: true }));
    const res = await host.api("/api/connect/media-ticket", { method: "POST" });
    expect(res.status).toBe(200);
    expect(host.renew).toHaveBeenCalledTimes(1);
    const sent = fetchMock.mock.calls.map(([, init]) => new Headers((init as RequestInit).headers).get("authorization"));
    expect(sent).toEqual(["Bearer old", "Bearer new"]);

    // No new session to be had: the 401 comes back, and the request isn't sent again.
    host.renew = vi.fn(async () => false);
    fetchMock.mockReset().mockResolvedValue(json(401, {}));
    expect((await host.api("/api/timeline")).status).toBe(401);
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });

  it("goes back to the page's own origin when the computer is left", () => {
    const host = new Host();
    host.use({ base: "https://edit-1.linker.cool", session: "s" });
    host.use(null);
    expect(host.web).toBe(false);
    expect(host.base).toBe("http://127.0.0.1:4317");
  });
});

describe("helloAt", () => {
  it("knows a Linker-Edit from a page that only serves files", async () => {
    const hello = { app: "linker-edit", name: "Studio-PC", version: "2026.11.14", protocol: 1 };
    expect(await helloAt("", 1500, async () => json(200, hello))).toEqual(hello);
    // Cloudflare's single-page fallback answers every path with the page itself.
    expect(await helloAt("", 1500, async () => new Response("<!doctype html>", { status: 200 }))).toBeNull();
    expect(await helloAt("", 1500, async () => json(404, {}))).toBeNull();
    expect(await helloAt("", 1500, async () => json(200, { app: "something-else", name: "x", version: "1" }))).toBeNull();
    expect(
      await helloAt("", 1500, async () => {
        throw new TypeError("Failed to fetch");
      }),
    ).toBeNull();
  });
});
