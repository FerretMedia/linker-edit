// The desktop app's updates as this server sees them. The app (desktop/main.ts) starts this server and sends it
// its update state; the server shows it to every page, this computer's and the user's other computers', and asks
// the app to restart into a new release once nothing is busy with Resolve. A restart in the middle of a turn
// would stop the agent halfway through an edit.
import type { AppUpdate, DesktopToServer, DesktopUpdateState, ServerToDesktop } from "../shared/protocol.ts";

export interface AppUpdatesOptions {
  /** To the desktop app. */
  send(message: ServerToDesktop): void;
  /** Whether a turn, render or open is using Resolve. */
  busy(): boolean;
  /** How often to look whether Resolve is free, while waiting. */
  pollMs?: number;
}

export type AppUpdates = ReturnType<typeof createAppUpdates>;

export function createAppUpdates(options: AppUpdatesOptions) {
  const pollMs = options.pollMs ?? 2000;
  let state: DesktopUpdateState | null = null;
  let waiting: ReturnType<typeof setInterval> | null = null;
  const listeners = new Set<() => void>();
  const notify = () => listeners.forEach((listener) => listener());

  function stopWaiting() {
    if (waiting) clearInterval(waiting);
    waiting = null;
  }

  function installNow() {
    stopWaiting();
    options.send({ type: "install_update" });
    notify();
  }

  function install() {
    if (state?.status !== "downloaded") return;
    if (!options.busy()) return installNow();
    if (waiting) return;
    waiting = setInterval(() => {
      if (!options.busy()) installNow();
    }, pollMs);
    notify();
  }

  return {
    status(): AppUpdate | null {
      return state && { ...state, waiting: waiting !== null };
    },
    onStatus(listener: () => void) {
      listeners.add(listener);
    },
    receive(message: DesktopToServer) {
      if (message.type === "install_requested") return install();
      state = message.state;
      if (state.status !== "downloaded") stopWaiting();
      notify();
    },
    check() {
      options.send({ type: "check_update" });
    },
    install,
    cancel() {
      if (!waiting) return;
      stopWaiting();
      notify();
    },
  };
}
