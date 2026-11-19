import { describe, expect, it } from "vitest";
import type { LinkerService } from "../linker.ts";
import { createWebOrigins } from "./webOrigins.ts";

const APP = "https://app.example";
const JANE = "https://linker-edit-web.jane.workers.dev";

function setup() {
  const listeners = new Set<() => void>();
  const state = { signedIn: true, userId: "user_jane", allowed: { user_jane: [JANE], user_bob: ["https://bob.example"] } as Record<string, string[]>, down: false };
  let clock = 1_000_000;
  const calls: string[] = [];
  const linker = {
    status: () => (state.signedIn ? { state: "signed_in" } : { state: "signed_out", problem: null }),
    onStatus: (listener: () => void) => void listeners.add(listener),
    accessToken: async () => (state.signedIn ? `token-${state.userId}` : null),
    userId: () => (state.signedIn ? state.userId : null),
  } as unknown as LinkerService;
  const fetchFn = (async (input: string | URL, init?: RequestInit) => {
    calls.push(String(input));
    if (state.down) throw new TypeError("fetch failed");
    const auth = (init?.headers as Record<string, string>).authorization;
    const user = auth.replace("Bearer token-", "");
    return new Response(JSON.stringify({ origins: state.allowed[user] ?? [] }), { status: 200 });
  }) as typeof fetch;
  const origins = createWebOrigins({ linker, appUrl: APP, fetch: fetchFn, now: () => clock });
  return {
    origins,
    state,
    calls,
    tick: (ms: number) => void (clock += ms),
    signOut() {
      state.signedIn = false;
      for (const listener of listeners) listener();
    },
  };
}

describe("allowed web addresses", () => {
  it("allows only the addresses the account here allowed at Linker, kept for 5 minutes", async () => {
    const { origins, calls, tick } = setup();
    expect(await origins.allows(JANE)).toBe(true);
    expect(calls).toEqual([`${APP}/api/edit/web/origins`]);
    expect(await origins.allows(JANE)).toBe(true);
    expect(await origins.allows("https://bob.example")).toBe(false);
    expect(await origins.allows(undefined)).toBe(false);
    expect(await origins.allows("null")).toBe(false);
    expect(calls).toHaveLength(1);
    tick(5 * 60_000);
    expect(await origins.allows(JANE)).toBe(true);
    expect(calls).toHaveLength(2);
  });

  it("asks again for an address it doesn't know, at most every 30 seconds", async () => {
    const { origins, state, calls, tick } = setup();
    expect(await origins.allows("https://new.example")).toBe(false);
    expect(await origins.allows("https://new.example")).toBe(false);
    expect(await origins.allows("https://other.example")).toBe(false);
    expect(calls).toHaveLength(1);
    // Allowed at Linker a moment ago: works on the next try after 30 seconds.
    state.allowed.user_jane!.push("https://new.example");
    tick(30_000);
    expect(await origins.allows("https://new.example")).toBe(true);
    expect(calls).toHaveLength(2);
    // Removed at Linker: gone with the next list.
    state.allowed.user_jane = [];
    tick(5 * 60_000);
    expect(await origins.allows(JANE)).toBe(false);
  });

  it("allows nothing signed out, and never another account's addresses", async () => {
    const { origins, state, signOut } = setup();
    expect(await origins.allows(JANE)).toBe(true);
    state.userId = "user_bob";
    expect(await origins.allows(JANE)).toBe(false);
    expect(await origins.allows("https://bob.example")).toBe(true);
    signOut();
    expect(await origins.allows("https://bob.example")).toBe(false);
  });

  it("keeps this account's last list while Linker can't be reached", async () => {
    const { origins, state, tick } = setup();
    expect(await origins.allows(JANE)).toBe(true);
    state.down = true;
    tick(5 * 60_000);
    expect(await origins.allows(JANE)).toBe(true);
    state.userId = "user_bob";
    expect(await origins.allows(JANE)).toBe(false);
  });
});
