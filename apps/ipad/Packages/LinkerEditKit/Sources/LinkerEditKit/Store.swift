import Foundation
import Observation

/// Folds one server message into the client's copy of the app state, as the web UI's applyServerMessage
/// (apps/desktop/src/lib/appState.ts): a snapshot replaces it, meta replaces everything but the chat, and an
/// item of the open thread is added or replaced by id.
public func applyServerMessage(_ state: AppState?, _ message: ServerMessage) -> AppState? {
    switch message {
    case .snapshot(let snapshot):
        return snapshot
    case .meta(let meta):
        guard var state else { return nil }
        state.meta = meta
        return state
    case .item(let threadId, let item):
        // An item of a thread the client just left can still be on its way.
        guard var state, threadId == state.meta.threadId else { return state }
        if let index = state.items.firstIndex(where: { $0.id == item.id }) {
            state.items[index] = item
        } else {
            state.items.append(item)
        }
        return state
    case .refused, .toast, .projectFile, .ignored:
        return state
    }
}

public struct Toast: Identifiable, Hashable, Sendable {
    public let id: Int
    public let text: String
    public let tone: Tone
}

/// A message the server didn't send to the agent, to put back in the composer.
public struct Refused: Hashable, Sendable {
    public let threadId: String
    public let text: String
}

/// The host's state as this client knows it. The host owns it; this only applies what arrives.
@MainActor
@Observable
public final class AppStore {
    public private(set) var state: AppState?
    public private(set) var toasts: [Toast] = []
    public var refused: Refused?
    private var nextToast = 0

    public init(state: AppState? = nil) {
        self.state = state
    }

    public func apply(_ message: ServerMessage) {
        switch message {
        case .refused(let threadId, let text):
            refused = Refused(threadId: threadId, text: text)
        case .toast(let text, let tone):
            nextToast += 1
            toasts = toasts.filter { $0.text != text } + [Toast(id: nextToast, text: text, tone: tone)]
        default:
            state = applyServerMessage(state, message)
        }
    }

    public func dismiss(_ toast: Toast) {
        toasts.removeAll { $0.id == toast.id }
    }

    /// Forgets the host's state, as when switching to another host.
    public func reset() {
        state = nil
        toasts = []
        refused = nil
    }
}
