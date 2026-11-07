import type { AppState, ClientMessage, ServerMessage } from "#shared/protocol.ts";
import { applyServerMessage } from "./appState.ts";

/** One WebSocket to the local server, reconnecting; the server owns all state. */
class ServerConnection {
  state = $state<AppState | null>(null);
  connected = $state(false);
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
      this.state = applyServerMessage(this.state, JSON.parse(event.data as string) as ServerMessage);
    };
  }

  send(message: ClientMessage) {
    this.#socket?.send(JSON.stringify(message));
  }
}

export const server = new ServerConnection();
