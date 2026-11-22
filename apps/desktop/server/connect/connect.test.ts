import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import type { LinkedComputer, LinkerAccount } from "../../shared/protocol.ts";
import type { LinkerService } from "../linker.ts";
import type { StartTunnel, TunnelEvent } from "./cloudflared.ts";
import { createConnect, LIMIT } from "./connect.ts";
import { createSessions } from "./sessions.ts";

const APP = "https://app.example";

/** The Linker account on this computer: Jane, unless signed out. */
function fakeLinker() {
  const listeners = new Set<() => void>();
  const fake = {
    state: "signed_in" as LinkerAccount["state"],
    userId: "user_jane" as string | null,
    tokens: { "token-jane": "user_jane", "token-bob": "user_bob", lew_jane: "user_jane", lew_bob: "user_bob" } as Record<string, string>,
    offline: false,
    signOut() {
      fake.state = "signed_out";
      for (const listener of listeners) listener();
    },
  };
  const service = {
    status: () =>
      fake.state === "signed_in"
        ? { state: "signed_in", name: "Jane", email: null, picture: null, plan: null, appUrl: APP }
        : { state: fake.state, problem: null },
    onStatus: (listener: () => void) => void listeners.add(listener),
    accessToken: async () => (fake.state === "signed_in" ? "token-jane" : null),
    userId: () => (fake.state === "signed_in" ? fake.userId : null),
    refresh: async () => {},
    whoIs: async (token: string) => {
      if (fake.offline) throw new Error("offline");
      return fake.tokens[token] ?? null;
    },
  } as unknown as LinkerService;
  return { fake, service };
}

/** Linker's /api/edit/computers, for Jane, with others' computers counted too; and the other computers' tunnels. */
function fakeRelay() {
  const computers = new Map<string, LinkedComputer & { owner: string; port: number }>();
  const calls: string[] = [];
  /** What each PUT sent. */
  const links: { tailnetUrl: string | null; tunnel: boolean }[] = [];
  let down = false;
  const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
  const mine = () => [...computers.values()].filter((c) => c.owner === "user_jane").map(({ owner: _o, port: _p, ...c }) => c);
  const fetchFn = (async (input: string | URL, init?: RequestInit) => {
    const url = new URL(String(input));
    const method = init?.method ?? "GET";
    calls.push(`${method} ${url.href}`);
    if (down) throw new TypeError("fetch failed");
    if (url.origin.startsWith("https://edit-") && url.pathname === "/api/connect/session") {
      return url.origin === "https://edit-0000000000000002.linker.example" ? json({ code: "one-time" }) : json({ error: "Not yours." }, 403);
    }
    if (url.origin.startsWith("https://edit-") && url.pathname === "/api/connect/native") {
      const { code } = JSON.parse(String(init?.body)) as { code: string };
      return code === "one-time" ? json({ session: "s-office", expiresAt: "2026-12-16T10:00:00.000Z" }) : json({ error: "Ran out." }, 401);
    }
    const auth = (init?.headers as Record<string, string> | undefined)?.authorization;
    if (auth !== "Bearer token-jane") return json({ error: "unauthorized" }, 401);
    const id = decodeURIComponent(url.pathname.replace("/api/edit/computers", "").replace(/^\//, ""));
    if (method === "GET") return json({ limit: LIMIT, computers: mine() });
    if (method === "PUT") {
      const body = JSON.parse(String(init?.body)) as { name: string; port: number; tailnetUrl: string | null; tunnel: boolean };
      links.push({ tailnetUrl: body.tailnetUrl, tunnel: body.tunnel });
      const existing = computers.get(id);
      if (existing && existing.owner !== "user_jane") return json({ error: "taken" }, 409);
      if (!existing && mine().length >= LIMIT) return json({ error: "limit", limit: LIMIT, computers: mine() }, 409);
      const tunnelUrl = `https://edit-${String(computers.size + 1).padStart(16, "0")}.linker.example`;
      const computer = existing ?? {
        id,
        owner: "user_jane",
        url: tunnelUrl,
        tailnetUrl: null,
        online: true,
        linkedAt: "2026-11-16T10:00:00.000Z",
        lastSeenAt: null,
        name: body.name,
        port: body.port,
      };
      // Without a tunnel Linker removes it, and the computer has no address there.
      const url = body.tunnel ? (computer.url ?? tunnelUrl) : null;
      Object.assign(computer, { name: body.name, port: body.port, tailnetUrl: body.tailnetUrl, url });
      computers.set(id, computer);
      const { owner: _o, port: _p, ...shown } = computer;
      return json({ computer: shown, tunnelToken: body.tunnel ? `tunnel-${id}` : null });
    }
    if (method === "DELETE") return computers.delete(id) ? new Response(null, { status: 204 }) : json({ error: "not_found" }, 404);
    return json({ error: "?" }, 405);
  }) as typeof fetch;
  return {
    computers,
    calls,
    links,
    fetchFn,
    setDown: (value: boolean) => void (down = value),
    addOther(owner: string, id: string) {
      const n = computers.size + 1;
      computers.set(id, {
        id,
        owner,
        name: `PC ${n}`,
        url: `https://edit-${String(n).padStart(16, "0")}.linker.example`,
        tailnetUrl: null,
        online: true,
        linkedAt: "2026-11-15T10:00:00.000Z",
        lastSeenAt: null,
        port: 4317,
      });
    },
  };
}

/** cloudflared that does nothing, with its log lines played by the test. */
function fakeTunnels() {
  const runs: { token: string; stopped: boolean; emit: (event: TunnelEvent) => void }[] = [];
  const start: StartTunnel = (_file, token, onEvent) => {
    const run = { token, stopped: false, emit: onEvent };
    runs.push(run);
    return { stop: () => void (run.stopped = true) };
  };
  return { runs, start };
}

let dir: string;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-connect-"));
});

function setup() {
  /** Tailscale's remote access here: its address while on, null while off, undefined while not known yet. */
  const tailscale = { url: null as string | null | undefined };
  const linker = fakeLinker();
  const relay = fakeRelay();
  const tunnels = fakeTunnels();
  const sessions = createSessions({ file: path.join(dir, "connect-sessions.json") });
  const make = () =>
    createConnect({
      file: path.join(dir, "connect.json"),
      port: 4317,
      linker: linker.service,
      appUrl: APP,
      sessions,
      prepare: async (onPercent) => {
        onPercent(100);
        return "cloudflared";
      },
      startTunnel: tunnels.start,
      tailnetUrl: () => tailscale.url,
      fetch: relay.fetchFn,
      name: "Studio",
    });
  return { linker, relay, tunnels, sessions, tailscale, make, connect: make() };
}

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

describe("Linker Connect", () => {
  it("links this computer and runs its tunnel", async () => {
    const { connect, relay, tunnels } = setup();
    expect(connect.status()).toMatchObject({ state: "off", url: null });
    await connect.setEnabled(true);
    expect(relay.calls).toContain(`PUT ${APP}/api/edit/computers/${connect.status().computerId}`);
    expect(tunnels.runs).toHaveLength(1);
    expect(tunnels.runs[0]!.token).toBe(`tunnel-${connect.status().computerId}`);
    expect(connect.status()).toMatchObject({ state: "on", connected: false, url: "https://edit-0000000000000001.linker.example" });
    tunnels.runs[0]!.emit({ state: "up" });
    expect(connect.status().connected).toBe(true);
    expect(connect.host()).toBe("edit-0000000000000001.linker.example");
    // The tunnel token stays in a file only this user reads, and the next start reuses the link.
    expect(fs.readFileSync(path.join(dir, "connect.json"), "utf8")).toContain("tunnel-");
  });

  it("stops at three computers, until one is unlinked", async () => {
    const { connect, relay, tunnels } = setup();
    relay.addOther("user_jane", "a");
    relay.addOther("user_jane", "b");
    relay.addOther("user_jane", "c");
    await connect.setEnabled(true);
    expect(tunnels.runs).toHaveLength(0);
    expect(connect.status()).toMatchObject({ state: "off", atLimit: true, problem: expect.stringContaining("3 computers") });
    expect(connect.status().computers?.map((c) => c.id)).toEqual(["a", "b", "c"]);
    await connect.unlink("b");
    expect(connect.status()).toMatchObject({ atLimit: false, problem: null });
    await connect.setEnabled(true);
    expect(connect.status().state).toBe("on");
    expect(connect.status().computers).toHaveLength(3);
  });

  it("restarts a tunnel that stopped by itself, and not one turned off", async () => {
    const { connect, tunnels } = setup();
    await connect.setEnabled(true);
    tunnels.runs[0]!.emit({ state: "down", detail: "lost" });
    expect(connect.status()).toMatchObject({ connected: false, problem: "The tunnel stopped: lost" });
    await connect.setEnabled(false);
    expect(connect.status().state).toBe("off");
    await new Promise((resolve) => setTimeout(resolve, 2_100));
    expect(tunnels.runs).toHaveLength(1);
  });

  it("lets in only the account that linked it", async () => {
    const { connect, linker, sessions } = setup();
    expect(await connect.admit("token-jane", "Laptop")).toEqual({ status: 503, error: expect.any(String) });
    await connect.setEnabled(true);
    const admitted = await connect.admit("token-jane", "Laptop");
    expect(admitted).toEqual({ code: expect.any(String) });
    expect(sessions.enter((admitted as { code: string }).code)).not.toBeNull();
    expect(await connect.admit("token-bob", "Bob's")).toEqual({ status: 403, error: expect.stringContaining("another Linker account") });
    // A web token from a page on your own web address counts the same way.
    expect(await connect.admit("lew_jane", "Browser")).toEqual({ code: expect.any(String) });
    expect(await connect.admit("lew_bob", "Bob's browser")).toEqual({ status: 403, error: expect.stringContaining("another Linker account") });
    expect(await connect.admit("stolen", "?")).toEqual({ status: 401, error: expect.any(String) });
    linker.fake.offline = true;
    expect(await connect.admit("token-jane", "Laptop")).toEqual({ status: 502, error: expect.any(String) });
  });

  it("gives the iPad app a Bearer session for a code, once, and only while on", async () => {
    const { connect, sessions } = setup();
    expect(connect.redeem("anything")).toEqual({ status: 503, error: expect.any(String) });
    await connect.setEnabled(true);
    const { code } = (await connect.admit("token-jane", "iPad")) as { code: string };
    const redeemed = connect.redeem(code);
    expect(redeemed).toEqual({ session: expect.any(String), expiresAt: expect.stringMatching(/^\d{4}-\d\d-\d\dT.*Z$/) });
    const { session } = redeemed as { session: string };
    expect(sessions.check(session, "native")).toEqual(expect.any(String));
    expect(sessions.list()).toEqual([expect.objectContaining({ name: "iPad", kind: "native" })]);
  });

  it("allows only so many tries a minute at each way in", async () => {
    const { connect } = setup();
    await connect.setEnabled(true);
    for (let i = 0; i < 20; i++) expect(connect.redeem("guess")).toMatchObject({ status: 401 });
    expect(connect.redeem("guess")).toEqual({ status: 429, error: expect.any(String) });
    // Guessing codes doesn't lock you out of asking for one.
    expect(await connect.admit("token-jane", "iPad")).toEqual({ code: expect.any(String) });
  });

  it("signs every device out when another Linker account links this computer", async () => {
    const first = setup();
    await first.connect.setEnabled(true);
    const native = first.sessions.enter(first.sessions.issueCode("iPad"), "native")!;
    // Restarted with Bob signed in here: Linker Connect comes back on for him, and drops Jane's devices first thing.
    first.linker.fake.userId = "user_bob";
    first.make().start();
    expect(first.sessions.check(native.token, "native")).toBeNull();
  });

  it("turns off and signs every device out when Linker is signed out", async () => {
    const { connect, linker, sessions, tunnels } = setup();
    await connect.setEnabled(true);
    const { token } = sessions.enter(sessions.issueCode("Laptop"))!;
    linker.fake.signOut();
    expect(tunnels.runs[0]!.stopped).toBe(true);
    expect(sessions.check(token)).toBeNull();
    expect(connect.status()).toMatchObject({ state: "signed_out", problem: expect.stringContaining("signed out") });
    expect(connect.host()).toBeNull();
  });

  it("comes back on at start, even offline with a link from before", async () => {
    const first = setup();
    await first.connect.setEnabled(true);
    first.relay.setDown(true);
    const again = first.make();
    again.start();
    await settle();
    await settle();
    expect(again.status().state).toBe("on");
    expect(first.tunnels.runs).toHaveLength(2);
  });

  it("opens your other computers with a one-time code, or signs in there with a session, and lists only those", async () => {
    const { connect, relay, linker } = setup();
    await connect.setEnabled(true);
    relay.addOther("user_jane", "office");
    relay.addOther("user_bob", "bobs");
    const others = await connect.others();
    expect(others.map((c) => c.id)).toEqual([]);
    // The list is kept for 30 seconds; the settings page asks for a fresh one.
    await connect.refresh();
    expect((await connect.others()).map((c) => c.id)).toEqual(["office"]);
    expect(await connect.open("https://edit-0000000000000002.linker.example/")).toBe(
      "https://edit-0000000000000002.linker.example/api/connect/enter?code=one-time",
    );
    await expect(connect.open("https://edit-0000000000000003.linker.example")).rejects.toThrow("isn't linked");
    // This computer's server passing the window's requests on signs in the way the iPad app does.
    expect(await connect.sessionOn("https://edit-0000000000000002.linker.example")).toEqual({
      session: "s-office",
      expiresAt: "2026-12-16T10:00:00.000Z",
    });
    expect(relay.calls.slice(-2)).toEqual([
      "POST https://edit-0000000000000002.linker.example/api/connect/session",
      "POST https://edit-0000000000000002.linker.example/api/connect/native",
    ]);
    // Kept for next time, so a restart doesn't add another device there; asked again once turned down.
    const calls = relay.calls.length;
    expect((await connect.sessionOn("https://edit-0000000000000002.linker.example"))?.session).toBe("s-office");
    expect(relay.calls.length).toBe(calls);
    await connect.sessionOn("https://edit-0000000000000002.linker.example", "s-office");
    expect(relay.calls.length).toBe(calls + 2);
    // Never shows the Linker token to a computer that isn't linked to the account.
    await expect(connect.sessionOn("https://edit-0000000000000003.linker.example")).rejects.toThrow("isn't linked");
    linker.fake.signOut();
    expect(await connect.sessionOn("https://edit-0000000000000002.linker.example")).toBeNull();
  });

  it("links for Tailscale only without a tunnel, and lets the owner in there", async () => {
    const { connect, relay, tunnels, tailscale, sessions } = setup();
    tailscale.url = "https://studio.tail1234.ts.net:8443";
    await connect.setTunnel(false);
    expect(connect.status()).toMatchObject({ state: "off", tunnel: false });
    await connect.setEnabled(true);
    expect(relay.links).toEqual([{ tailnetUrl: "https://studio.tail1234.ts.net:8443", tunnel: false }]);
    expect(tunnels.runs).toHaveLength(0);
    expect(connect.status()).toMatchObject({ state: "on", tunnel: false, url: null });
    expect(connect.host()).toBeNull();
    expect(connect.status().computers).toEqual([expect.objectContaining({ url: null, tailnetUrl: "https://studio.tail1234.ts.net:8443" })]);
    // Signing in works without the tunnel: over Tailscale, still only for the account that linked it.
    const admitted = await connect.admit("token-jane", "Browser on Windows");
    expect(admitted).toEqual({ code: expect.any(String) });
    expect(connect.redeem((admitted as { code: string }).code)).toEqual({ session: expect.any(String), expiresAt: expect.any(String) });
    expect(await connect.admit("token-bob", "Bob's")).toMatchObject({ status: 403 });
    // Turning the tunnel back on runs it, and devices stay signed in.
    await connect.setTunnel(true);
    expect(relay.links.at(-1)).toEqual({ tailnetUrl: "https://studio.tail1234.ts.net:8443", tunnel: true });
    expect(tunnels.runs).toHaveLength(1);
    expect(connect.status()).toMatchObject({ state: "on", tunnel: true, url: expect.stringMatching(/^https:\/\/edit-/) });
    expect(sessions.list()).toHaveLength(1);
    // And off again stops it.
    await connect.setTunnel(false);
    expect(tunnels.runs[0]!.stopped).toBe(true);
    expect(connect.status()).toMatchObject({ state: "on", url: null });
    await connect.setEnabled(false);
    expect(await connect.admit("token-jane", "Browser")).toMatchObject({ status: 503 });
  });

  it("tells Linker its Tailscale address again when remote access changes while linked", async () => {
    const { connect, relay, tailscale } = setup();
    // Not known yet at start: nothing is sent for it until it is.
    tailscale.url = undefined;
    await connect.setEnabled(true);
    expect(relay.links).toEqual([{ tailnetUrl: null, tunnel: true }]);
    connect.tailnetChanged();
    expect(relay.links).toHaveLength(1);
    tailscale.url = "https://studio.tail1234.ts.net:8443";
    connect.tailnetChanged();
    await settle();
    await settle();
    expect(relay.links.at(-1)).toEqual({ tailnetUrl: "https://studio.tail1234.ts.net:8443", tunnel: true });
    // Unchanged: nothing sent.
    connect.tailnetChanged();
    expect(relay.links).toHaveLength(2);
    tailscale.url = null;
    connect.tailnetChanged();
    await settle();
    await settle();
    expect(relay.links.at(-1)).toEqual({ tailnetUrl: null, tunnel: true });
    // Off: Linker hears it at the next link.
    await connect.setEnabled(false);
    tailscale.url = "https://studio.tail1234.ts.net:8443";
    connect.tailnetChanged();
    expect(relay.links).toHaveLength(3);
  });

  it("opens only computers with an address at Linker", async () => {
    const { connect, relay } = setup();
    await connect.setEnabled(true);
    relay.addOther("user_jane", "office");
    relay.computers.get("office")!.url = null;
    await connect.refresh();
    expect((await connect.others()).map((c) => [c.id, c.url])).toEqual([["office", null]]);
    await expect(connect.open("https://edit-0000000000000002.linker.example")).rejects.toThrow("isn't linked");
  });

  it("unlinking this computer turns it off and forgets its address", async () => {
    const { connect, tunnels, sessions } = setup();
    await connect.setEnabled(true);
    const { code } = (await connect.admit("token-jane", "iPad")) as { code: string };
    const { session } = connect.redeem(code) as { session: string };
    await connect.unlink(connect.status().computerId);
    expect(tunnels.runs[0]!.stopped).toBe(true);
    expect(sessions.check(session, "native")).toBeNull();
    expect(connect.status()).toMatchObject({ state: "off", url: null, computers: [] });
  });
});
