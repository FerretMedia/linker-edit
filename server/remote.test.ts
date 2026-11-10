import { describe, expect, it } from "vitest";
import { createRemoteAccess, requestSource, type RunTailscale, type TailscaleResult } from "./remote.ts";

const gate = { host: "studio.tail1234.ts.net:8443", owner: "Jane@example.com" };
const viaServe = { "x-forwarded-for": "100.101.102.103", "tailscale-user-login": "jane@example.com" };

describe("requestSource", () => {
  it.each([
    ["this computer", { host: "127.0.0.1:4317" }, gate, "local"],
    ["the dev server", { host: "localhost:5317" }, null, "local"],
    ["the owner over Serve", { host: "studio.tail1234.ts.net:8443", ...viaServe }, gate, "remote"],
    ["the owner, with Serve's forwarded host", { host: "127.0.0.1:4317", "x-forwarded-host": "studio.tail1234.ts.net:8443", ...viaServe }, gate, "remote"],
    ["the owner while remote access is off", { host: "studio.tail1234.ts.net:8443", ...viaServe }, null, null],
    ["another account", { host: "studio.tail1234.ts.net:8443", ...viaServe, "tailscale-user-login": "bob@example.com" }, gate, null],
    ["a tagged device, which has no login", { host: "studio.tail1234.ts.net:8443", "x-forwarded-for": "100.64.0.9" }, gate, null],
    ["a proxied request that claims to be local", { host: "127.0.0.1:4317", ...viaServe }, gate, null],
    ["another Serve port", { host: "studio.tail1234.ts.net", ...viaServe }, gate, null],
    ["a rebound site", { host: "evil.example:4317" }, gate, null],
    ["a rebound site faking Serve", { host: "evil.example:4317", ...viaServe }, gate, null],
    ["no Host", {}, gate, null],
  ])("%s", (_name, headers, current, expected) => {
    expect(requestSource(headers, current)).toBe(expected);
  });
});

const ok = (stdout: string): TailscaleResult => ({ code: 0, stdout, stderr: "", timedOut: false });

const runningStatus = JSON.stringify({
  BackendState: "Running",
  Self: { DNSName: "Studio.tail1234.ts.net.", UserID: 7, CapMap: { https: null } },
  User: { "7": { LoginName: "jane@example.com" } },
});

/** A tailscale CLI that keeps its Serve config, and records what it was asked. */
function fakeTailscale(serve: Record<string, unknown> = {}, status = runningStatus) {
  const calls: string[] = [];
  let config = serve;
  const run: RunTailscale = async (args) => {
    calls.push(args.join(" "));
    if (args[0] === "status") return ok(status);
    if (args.join(" ") === "serve status --json") return ok(JSON.stringify(config));
    if (args.includes("off")) config = {};
    else config = { Web: { "studio.tail1234.ts.net:8443": { Handlers: { "/": { Proxy: args.at(-1) } } } } };
    return ok("");
  };
  return { run, calls };
}

describe("createRemoteAccess", () => {
  it("turns Tailscale Serve on and off for this server", async () => {
    const tailscale = fakeTailscale();
    const remote = createRemoteAccess({ port: 4317, servePort: 8443, run: tailscale.run });
    await remote.refresh();
    expect(remote.status()).toEqual({ state: "off", url: "https://studio.tail1234.ts.net:8443", problem: null });
    expect(remote.gate()).toBeNull();

    await remote.setEnabled(true);
    expect(tailscale.calls).toContain("serve --bg --https=8443 http://127.0.0.1:4317");
    expect(remote.status()).toEqual({ state: "on", url: "https://studio.tail1234.ts.net:8443", owner: "jane@example.com" });
    expect(remote.gate()).toEqual({ host: "studio.tail1234.ts.net:8443", owner: "jane@example.com" });

    await remote.setEnabled(false);
    expect(tailscale.calls).toContain("serve --https=8443 off");
    expect(remote.status().state).toBe("off");
    expect(remote.gate()).toBeNull();
  });

  it("finds it already on after a restart, as Tailscale keeps Serve on", async () => {
    const tailscale = fakeTailscale({ Web: { "studio.tail1234.ts.net:8443": { Handlers: { "/": { Proxy: "127.0.0.1:4317" } } } } });
    const remote = createRemoteAccess({ port: 4317, servePort: 8443, run: tailscale.run });
    await remote.refresh();
    expect(remote.status().state).toBe("on");
  });

  it("never takes over a port that serves something else", async () => {
    const tailscale = fakeTailscale({ Web: { "studio.tail1234.ts.net:8443": { Handlers: { "/": { Proxy: "http://127.0.0.1:3000" } } } } });
    const remote = createRemoteAccess({ port: 4317, servePort: 8443, run: tailscale.run });
    await remote.refresh();
    expect(remote.status()).toMatchObject({ state: "off", problem: expect.stringContaining('Set "tailscalePort"') });
    await remote.setEnabled(true);
    expect(tailscale.calls.filter((call) => call.startsWith("serve --bg"))).toEqual([]);
  });

  it.each([
    ["isn't installed", null, "Install Tailscale"],
    ["is logged out", ok(JSON.stringify({ BackendState: "NeedsLogin" })), "Log in to Tailscale"],
    ["isn't running", { code: 1, stdout: "", stderr: "failed to connect to local Tailscale service", timedOut: false }, "Start Tailscale"],
  ])("says what to do when Tailscale %s", async (_name, result, hint) => {
    const remote = createRemoteAccess({ port: 4317, servePort: 8443, run: async () => result });
    await remote.refresh();
    expect(remote.status()).toMatchObject({ state: "unavailable", hint: expect.stringContaining(hint) });
  });

  it("shows Tailscale's link when Serve waits for the tailnet to allow it", async () => {
    const tailscale = fakeTailscale();
    const run: RunTailscale = async (args, timeout) =>
      args[1] === "--bg"
        ? { code: 1, stdout: "Serve is not enabled on your tailnet.\nTo enable, visit:\n\n https://login.tailscale.com/f/serve?node=abc\n", stderr: "", timedOut: true }
        : tailscale.run(args, timeout);
    const remote = createRemoteAccess({ port: 4317, servePort: 8443, run });
    await remote.refresh();
    await remote.setEnabled(true);
    expect(remote.status()).toMatchObject({ state: "off", problem: expect.stringContaining("https://login.tailscale.com/f/serve?node=abc") });
  });
});
