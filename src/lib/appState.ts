import type { AppState, ServerMessage } from "#shared/protocol.ts";

/** Folds one server message into the client's copy of the app state, in place where it can. */
export function applyServerMessage(state: AppState | null, message: ServerMessage): AppState | null {
  if (message.type === "snapshot") return message.state;
  if (!state) return state;
  if (message.type === "meta") {
    Object.assign(state, message.meta);
    return state;
  }
  // An item of a thread the client just left can still be on its way.
  if (message.type !== "item" || message.threadId !== state.threadId) return state;
  const index = state.items.findIndex((item) => item.id === message.item.id);
  if (index === -1) state.items.push(message.item);
  else state.items[index] = message.item;
  return state;
}
