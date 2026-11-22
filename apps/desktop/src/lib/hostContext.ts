import { getContext, setContext } from "svelte";
import { Host } from "./host.ts";

const KEY = Symbol("host");
let pageOwn: Host | null = null;

/**
 * The server the components below talk to: the open thread's machine, or the one the settings page shows. A getter,
 * so they follow when it changes.
 */
export function provideHost(get: () => Host) {
  setContext(KEY, get);
}

/** The provided server; without one, the page's own. */
export function useHost(): () => Host {
  return getContext<(() => Host) | undefined>(KEY) ?? (() => (pageOwn ??= new Host()));
}
