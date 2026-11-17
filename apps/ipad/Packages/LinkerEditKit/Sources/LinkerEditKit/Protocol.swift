import Foundation

// What the iPad reads and sends over /ws: the part of apps/desktop/shared/protocol.ts it uses. Field names are
// the server's. Tests decode the messages recorded from the real server (Tests/Fixtures).

public struct Provider: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }
    public static let claude = Provider(rawValue: "claude")
    public static let codex = Provider(rawValue: "codex")

    public var label: String {
        switch rawValue {
        case "claude": return "Claude"
        case "codex": return "Codex"
        default: return rawValue.capitalized
        }
    }
}

/// "auto" is full access (the default), "ask" is supervised.
public struct ApprovalMode: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }
    public static let auto = ApprovalMode(rawValue: "auto")
    public static let ask = ApprovalMode(rawValue: "ask")
}

public struct ToolStatus: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }
    public static let running = ToolStatus(rawValue: "running")
    public static let done = ToolStatus(rawValue: "done")
    public static let error = ToolStatus(rawValue: "error")
}

public struct Tone: OpenEnum {
    public let rawValue: String
    public init(rawValue: String) { self.rawValue = rawValue }
    public static let info = Tone(rawValue: "info")
    public static let error = Tone(rawValue: "error")
}

public struct ToolCall: Hashable, Sendable {
    public var id: String
    public var name: String
    public var input: JSONValue
    public var status: ToolStatus
    public var output: String?

    public init(id: String, name: String, input: JSONValue, status: ToolStatus, output: String? = nil) {
        self.id = id
        self.name = name
        self.input = input
        self.status = status
        self.output = output
    }
}

public enum ChatItem: Decodable, Hashable, Identifiable, Sendable {
    case user(id: String, text: String)
    case assistant(id: String, text: String)
    case tool(ToolCall)
    case notice(id: String, text: String, tone: Tone)
    /// A kind this app doesn't know yet; shown as nothing.
    case unknown(id: String, kind: String)

    public var id: String {
        switch self {
        case .user(let id, _), .assistant(let id, _), .notice(let id, _, _), .unknown(let id, _): return id
        case .tool(let call): return call.id
        }
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        let id: String = try c.required("id")
        let kind: String = c.value("kind", "")
        switch kind {
        case "user": self = .user(id: id, text: c.value("text", ""))
        case "assistant": self = .assistant(id: id, text: c.value("text", ""))
        case "tool":
            self = .tool(ToolCall(
                id: id,
                name: c.value("name", ""),
                input: c.value("input", JSONValue.null),
                status: c.value("status", ToolStatus.running),
                output: c.optional("output")
            ))
        case "notice": self = .notice(id: id, text: c.value("text", ""), tone: c.value("tone", Tone.info))
        default: self = .unknown(id: id, kind: kind)
        }
    }
}

public struct ResolveDatabase: Decodable, Hashable, Sendable {
    public var type: String
    public var name: String

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        type = c.value("type", "")
        name = c.value("name", "")
    }
}

public enum ResolveStatus: Decodable, Hashable, Sendable {
    case checking
    case connected(product: String, version: String, project: String?, timeline: String?, database: ResolveDatabase?)
    case notRunning
    case unreachable(hint: String)
    case blocked(hint: String)
    case setupNeeded(hint: String)
    case unknown(state: String)

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        let state: String = c.value("state", "")
        switch state {
        case "checking": self = .checking
        case "connected":
            self = .connected(
                product: c.value("product", "DaVinci Resolve"),
                version: c.value("version", ""),
                project: c.optional("project"),
                timeline: c.optional("timeline"),
                database: c.optional("database")
            )
        case "not_running": self = .notRunning
        case "unreachable": self = .unreachable(hint: c.value("hint", ""))
        case "blocked": self = .blocked(hint: c.value("hint", ""))
        case "setup_needed": self = .setupNeeded(hint: c.value("hint", ""))
        default: self = .unknown(state: state)
        }
    }
}

/// Which Resolve project a Linker-Edit project belongs to.
public struct ResolveLink: Decodable, Hashable, Sendable {
    /// "local", "network" or "cloud".
    public var kind: String
    public var name: String

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        kind = c.value("kind", "")
        name = c.value("name", "")
    }
}

public struct ThreadSummary: Decodable, Hashable, Identifiable, Sendable {
    public var id: String
    public var title: String
    public var updatedAt: String
    public var archived: Bool
    public var busy: Bool
    /// An approval is waiting for the user.
    public var waiting: Bool
    public var pinnedAt: String?
    public var settledAt: String?
    public var lastReadAt: String?
    public var turnEndedAt: String?

    /// Its last turn ended after anyone last looked at it.
    public var unread: Bool {
        guard let ended = turnEndedAt else { return false }
        guard let read = lastReadAt else { return true }
        return ended > read
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        id = try c.required("id")
        title = c.value("title", "New thread")
        updatedAt = c.value("updatedAt", "")
        archived = c.value("archived", false)
        busy = c.value("busy", false)
        waiting = c.value("waiting", false)
        pinnedAt = c.optional("pinnedAt")
        settledAt = c.optional("settledAt")
        lastReadAt = c.optional("lastReadAt")
        turnEndedAt = c.optional("turnEndedAt")
    }
}

public struct ProjectSummary: Decodable, Hashable, Identifiable, Sendable {
    public var id: String
    public var name: String
    public var dir: String
    public var link: ResolveLink?
    /// Its Resolve project is the one open in Resolve now.
    public var open: Bool
    public var pinned: Bool
    public var archived: Bool
    /// Newest first, archived ones included.
    public var threads: [ThreadSummary]

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        id = try c.required("id")
        name = c.value("name", "")
        dir = c.value("dir", "")
        link = c.optional("link")
        open = c.value("open", false)
        pinned = c.value("pinned", false)
        archived = c.value("archived", false)
        threads = c.list("threads")
    }
}

public struct ApprovalRequest: Decodable, Hashable, Identifiable, Sendable {
    public var id: String
    public var tool: String
    public var input: JSONValue

    public init(id: String, tool: String, input: JSONValue) {
        self.id = id
        self.tool = tool
        self.input = input
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        id = try c.required("id")
        tool = c.value("tool", "")
        input = c.value("input", JSONValue.null)
    }
}

public struct PreviewTimeline: Decodable, Hashable, Sendable {
    public var id: String
    public var name: String

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        id = c.value("id", "")
        name = c.value("name", "")
    }
}

/// A small render of one timeline, made by Resolve; url is a path on the host, such as /preview/preview-1.mp4.
public enum PreviewState: Decodable, Hashable, Sendable {
    case idle
    case rendering(timeline: PreviewTimeline?, percent: Double)
    case ready(timeline: PreviewTimeline?, url: String, renderedAt: String, outdated: Bool)
    case error(message: String)
    case unknown(state: String)

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        let state: String = c.value("state", "")
        switch state {
        case "idle": self = .idle
        case "rendering": self = .rendering(timeline: c.optional("timeline"), percent: c.value("percent", 0))
        case "ready":
            self = .ready(
                timeline: c.optional("timeline"),
                url: c.value("url", ""),
                renderedAt: c.value("renderedAt", ""),
                outdated: c.value("outdated", false)
            )
        case "error": self = .error(message: c.value("message", "The preview failed."))
        default: self = .unknown(state: state)
        }
    }
}

/// How full the agent's context was after its latest request. Claude only.
public struct ContextUsage: Decodable, Hashable, Sendable {
    public var usedTokens: Int
    public var maxTokens: Int?

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        usedTokens = c.value("usedTokens", 0)
        maxTokens = c.optional("maxTokens")
    }
}

/// Why a message in this thread can't go to the agent right now.
public struct Blocked: Decodable, Hashable, Sendable {
    public var message: String
    public var canOpen: Bool

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        message = c.value("message", "")
        canOpen = c.value("canOpen", false)
    }
}

/// Everything in AppState but the chat: what a "meta" message replaces.
public struct AppMeta: Decodable, Hashable, Sendable {
    public var projects: [ProjectSummary]
    /// The thread this client is looking at; nil when there are no projects.
    public var threadId: String?
    public var projectId: String?
    public var provider: Provider
    public var defaultProvider: Provider
    public var approvalMode: ApprovalMode
    /// This thread's turn is running.
    public var busy: Bool
    public var approvals: [ApprovalRequest]
    public var resolve: ResolveStatus
    public var resolveHere: Bool
    public var version: String
    public var workspaceDir: String?
    public var preview: PreviewState
    public var context: ContextUsage?
    public var blocked: Blocked?
    /// Linker-Edit is opening this project in Resolve.
    public var opening: String?
    /// The name of the computer the host runs on.
    public var computer: String

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        projects = c.list("projects")
        threadId = c.optional("threadId")
        projectId = c.optional("projectId")
        provider = c.value("provider", Provider.claude)
        defaultProvider = c.value("defaultProvider", Provider.claude)
        approvalMode = c.value("approvalMode", ApprovalMode.auto)
        busy = c.value("busy", false)
        approvals = c.list("approvals")
        resolve = c.value("resolve", ResolveStatus.checking)
        resolveHere = c.value("resolveHere", true)
        version = c.value("version", "")
        workspaceDir = c.optional("workspaceDir")
        preview = c.value("preview", PreviewState.idle)
        context = c.optional("context")
        blocked = c.optional("blocked")
        opening = c.optional("opening")
        computer = c.value("computer", "")
    }

    /// The thread this client is looking at.
    public var thread: ThreadSummary? {
        guard let threadId else { return nil }
        for project in projects {
            if let thread = project.threads.first(where: { $0.id == threadId }) { return thread }
        }
        return nil
    }

    public var project: ProjectSummary? {
        projects.first { $0.id == projectId }
    }
}

/// What a snapshot carries: the meta and the open thread's chat.
public struct AppState: Decodable, Hashable, Sendable {
    public var meta: AppMeta
    public var items: [ChatItem]

    public init(meta: AppMeta, items: [ChatItem]) {
        self.meta = meta
        self.items = items
    }

    public init(from decoder: Decoder) throws {
        meta = try AppMeta(from: decoder)
        items = try decoder.keyed().list("items")
    }
}

public enum ServerMessage: Decodable, Sendable {
    case snapshot(AppState)
    case meta(AppMeta)
    case item(threadId: String, item: ChatItem)
    /// The message wasn't sent; it goes back in the composer.
    case refused(threadId: String, text: String)
    case toast(text: String, tone: Tone)
    /// A message only the web UI uses (history, library, folders…), or one this app doesn't know.
    case ignored(type: String)

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        let type: String = try c.required("type")
        switch type {
        case "snapshot": self = try .snapshot(c.required("state"))
        case "meta": self = try .meta(c.required("meta"))
        case "item": self = try .item(threadId: c.required("threadId"), item: c.required("item"))
        case "refused": self = .refused(threadId: c.value("threadId", ""), text: c.value("text", ""))
        case "toast": self = .toast(text: c.value("text", ""), tone: c.value("tone", Tone.info))
        default: self = .ignored(type: type)
        }
    }

    public static func decode(_ data: Data) throws -> ServerMessage {
        try JSONDecoder().decode(ServerMessage.self, from: data)
    }
}

/// What the iPad sends. The same JSON as the web UI's; Tests/Fixtures/client holds the server's own examples.
public enum ClientMessage: Encodable, Equatable, Sendable {
    case send(text: String)
    case stop
    case approve(id: String, allow: Bool)
    case selectThread(threadId: String)
    case newThread(projectId: String)
    case setApprovalMode(ApprovalMode)
    case setProvider(Provider)
    case refreshResolve
    /// Without a timeline id, renders Resolve's current timeline.
    case renderPreview(timelineId: String?)
    case stopPreview
    case markRead(threadId: String, read: Bool)

    private enum Keys: String, CodingKey {
        case type, text, id, allow, threadId, projectId, mode, provider, timelineId, read
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: Keys.self)
        switch self {
        case .send(let text):
            try c.encode("send", forKey: .type)
            try c.encode(text, forKey: .text)
        case .stop:
            try c.encode("stop", forKey: .type)
        case .approve(let id, let allow):
            try c.encode("approve", forKey: .type)
            try c.encode(id, forKey: .id)
            try c.encode(allow, forKey: .allow)
        case .selectThread(let threadId):
            try c.encode("select_thread", forKey: .type)
            try c.encode(threadId, forKey: .threadId)
        case .newThread(let projectId):
            try c.encode("new_thread", forKey: .type)
            try c.encode(projectId, forKey: .projectId)
        case .setApprovalMode(let mode):
            try c.encode("set_approval_mode", forKey: .type)
            try c.encode(mode, forKey: .mode)
        case .setProvider(let provider):
            try c.encode("set_provider", forKey: .type)
            try c.encode(provider, forKey: .provider)
        case .refreshResolve:
            try c.encode("refresh_resolve", forKey: .type)
        case .renderPreview(let timelineId):
            try c.encode("render_preview", forKey: .type)
            try c.encodeIfPresent(timelineId, forKey: .timelineId)
        case .stopPreview:
            try c.encode("stop_preview", forKey: .type)
        case .markRead(let threadId, let read):
            try c.encode("mark_read", forKey: .type)
            try c.encode(threadId, forKey: .threadId)
            try c.encode(read, forKey: .read)
        }
    }

    public func json() throws -> String {
        String(decoding: try JSONEncoder().encode(self), as: UTF8.self)
    }
}
