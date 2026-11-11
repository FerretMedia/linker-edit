import type { AppState, ClientMessage, FolderListing, HistoryEntry, ResolveLibrary, ServerMessage } from "#shared/protocol.ts";
import { applyServerMessage } from "./appState.ts";

export interface Toast {
  id: number;
  text: string;
  tone: "info" | "error";
}

const TOAST_MS = 6000;

/** One WebSocket to the local server, reconnecting; the server owns all state. */
class ServerConnection {
  state = $state<AppState | null>(null);
  connected = $state(false);
  /** Null until the history panel first asks for it. */
  history = $state<{ projectId: string; entries: HistoryEntry[] } | null>(null);
  /** Null until the add-project dialog asks for it. */
  library = $state<ResolveLibrary | { error: string } | null>(null);
  toasts = $state<Toast[]>([]);
  /** A message the server didn't send to the agent, to put back in the composer. */
  onRefused: ((threadId: string, text: string) => void) | null = null;
  #socket: WebSocket | null = null;
  #toastId = 0;
  #folderRequest = 0;
  #folderReplies = new Map<number, (listing: FolderListing | { error: string }) => void>();

  connect() {
    if (this.#socket && this.#socket.readyState !== WebSocket.CLOSED) return;
    const socket = new WebSocket(`${location.protocol === "https:" ? "wss" : "ws"}://${location.host}/ws`);
    this.#socket = socket;
    socket.onopen = () => (this.connected = true);
    socket.onclose = () => {
      this.connected = false;
      for (const reply of this.#folderReplies.values()) reply({ error: "Lost the connection to the Linker-Edit server." });
      this.#folderReplies.clear();
      setTimeout(() => this.connect(), 1000);
    };
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data as string) as ServerMessage;
      if (message.type === "history") this.history = { projectId: message.projectId, entries: message.entries };
      else if (message.type === "library") this.library = message.library;
      else if (message.type === "folders") {
        this.#folderReplies.get(message.requestId)?.(message.listing);
        this.#folderReplies.delete(message.requestId);
      }
      else if (message.type === "refused") this.onRefused?.(message.threadId, message.text);
      else if (message.type === "toast") this.toast(message.text, message.tone);
      else this.state = applyServerMessage(this.state, message);
    };
  }

  send(message: ClientMessage) {
    this.#socket?.send(JSON.stringify(message));
  }

  /** The folders in a folder on the server's computer; an empty path is its home folder. */
  browseFolders(path: string): Promise<FolderListing | { error: string }> {
    if (this.#socket?.readyState !== WebSocket.OPEN) return Promise.resolve({ error: "Not connected to the Linker-Edit server." });
    const requestId = ++this.#folderRequest;
    return new Promise((resolve) => {
      this.#folderReplies.set(requestId, resolve);
      this.send({ type: "browse_folders", requestId, path });
    });
  }

  toast(text: string, tone: "info" | "error") {
    const id = ++this.#toastId;
    this.toasts = [...this.toasts.filter((toast) => toast.text !== text), { id, text, tone }];
    setTimeout(() => this.dismiss(id), TOAST_MS);
  }

  dismiss(id: number) {
    this.toasts = this.toasts.filter((toast) => toast.id !== id);
  }
}

export const server = new ServerConnection();
