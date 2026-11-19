import type { AppState, ClientMessage, FolderListing, HistoryEntry, LibraryView, ResolveLibrary, ServerMessage } from "#shared/protocol.ts";
import { applyServerMessage } from "./appState.ts";
import { host } from "./host.ts";

export interface Toast {
  id: number;
  text: string;
  tone: "info" | "error";
}

const TOAST_MS = 6000;

/** One WebSocket to the server, reconnecting; the server owns all state. */
class ServerConnection {
  state = $state<AppState | null>(null);
  connected = $state(false);
  /** Null until the history panel first asks for it. */
  history = $state<{ projectId: string; entries: HistoryEntry[] } | null>(null);
  /** A project's PROJECT.md; null until the project file panel first asks for it. */
  projectFile = $state<{ projectId: string; file: string; text: string } | null>(null);
  /** Null until the add-project dialog asks for it. */
  library = $state<ResolveLibrary | { error: string } | null>(null);
  /** The library page's library, while the page is open. */
  libraryView = $state<LibraryView | null>(null);
  toasts = $state<Toast[]>([]);
  /** A message the server didn't send to the agent, to put back in the composer. */
  onRefused: ((threadId: string, text: string) => void) | null = null;
  /** Web mode: checks the session before connecting again, as a browser hides why a socket was refused. */
  beforeReconnect: (() => Promise<void>) | null = null;
  #socket: WebSocket | null = null;
  /** The server's version when this page connected; a restart into a new release reloads the page. */
  #version: string | null = null;
  #toastId = 0;
  #folderRequest = 0;
  #folderReplies = new Map<number, (listing: FolderListing | { error: string }) => void>();

  connect() {
    if (this.#socket && this.#socket.readyState !== WebSocket.CLOSED) return;
    const { url, protocols } = host.ws("/ws?meta=patch");
    const socket = new WebSocket(url, protocols);
    this.#socket = socket;
    socket.onopen = () => (this.connected = true);
    socket.onclose = () => {
      // Closed by disconnect(): nothing to reconnect.
      if (this.#socket !== socket) return;
      this.connected = false;
      for (const reply of this.#folderReplies.values()) reply({ error: "Lost the connection to the Linker-Edit server." });
      this.#folderReplies.clear();
      setTimeout(() => {
        if (this.#socket !== socket) return;
        void (this.beforeReconnect?.() ?? Promise.resolve()).finally(() => this.#socket === socket && this.connect());
      }, 1000);
    };
    socket.onmessage = (event) => {
      if (this.#socket !== socket) return;
      const message = JSON.parse(event.data as string) as ServerMessage;
      if (message.type === "history") this.history = { projectId: message.projectId, entries: message.entries };
      else if (message.type === "project_file") this.projectFile = { projectId: message.projectId, file: message.file, text: message.text };
      else if (message.type === "library") this.library = message.library;
      else if (message.type === "library_view") this.libraryView = message.view;
      else if (message.type === "folders") {
        this.#folderReplies.get(message.requestId)?.(message.listing);
        this.#folderReplies.delete(message.requestId);
      }
      else if (message.type === "refused") this.onRefused?.(message.threadId, message.text);
      else if (message.type === "toast") this.toast(message.text, message.tone);
      else {
        if (message.type === "snapshot") {
          // This page's code came from the old version, as when another computer restarted to update. A web
          // address's page doesn't come from the computer, so reloading wouldn't update it.
          if (this.#version !== null && message.state.version !== this.#version && !host.web) {
            location.reload();
            return;
          }
          this.#version = message.state.version;
        }
        this.state = applyServerMessage(this.state, message);
      }
    };
  }

  /** Web mode, leaving a computer: closes the socket for good and forgets that computer's state. */
  disconnect() {
    const socket = this.#socket;
    this.#socket = null;
    socket?.close();
    for (const reply of this.#folderReplies.values()) reply({ error: "Lost the connection to the Linker-Edit server." });
    this.#folderReplies.clear();
    this.connected = false;
    this.#version = null;
    this.state = null;
    this.history = null;
    this.projectFile = null;
    this.library = null;
    this.libraryView = null;
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
