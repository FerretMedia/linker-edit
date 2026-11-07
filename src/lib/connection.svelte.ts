import type { AppState, ClientMessage, HistoryEntry, ServerMessage } from "#shared/protocol.ts";
import { applyServerMessage } from "./appState.ts";

/** One WebSocket to the local server, reconnecting; the server owns all state. */
class ServerConnection {
  state = $state<AppState | null>(null);
  connected = $state(false);
  /** Null until the history panel first asks for it. */
  history = $state<HistoryEntry[] | null>(null);
  #socket: WebSocket | null = null;

  connect() {
    if (this.#socket && this.#socket.readyState !== WebSocket.CLOSED) return;
    const socket = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
    this.#socket = socket;
    socket.onopen = () => (this.connected = true);
    socket.onclose = () => {
      this.connected = false;
      setTimeout(() => this.connect(), 1000);
    };
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data as string) as ServerMessage;
      if (message.type === "history") this.history = message.entries;
      else this.state = applyServerMessage(this.state, message);
    };
  }

  send(message: ClientMessage) {
    this.#socket?.send(JSON.stringify(message));
  }
}

export const server = new ServerConnection();
