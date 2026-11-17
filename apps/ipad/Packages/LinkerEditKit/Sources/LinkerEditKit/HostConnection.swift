import Foundation

public enum ConnectionEvent: Sendable {
    /// The first message arrived: the host let this client in.
    case connected
    case message(ServerMessage)
    /// The connection dropped or couldn't be made; another try follows. reason is set when it never opened.
    case disconnected(reason: String?)
    /// The host turned the connection away (401/403). No more tries: with a session it ran out or was signed
    /// out; without one, this device isn't let in.
    case refused(status: Int)
}

/// How long to wait before reconnecting: 1, 2, 4, 8, then every 10 seconds.
public struct RetryPolicy: Sendable {
    public var first: Double
    public var max: Double

    public init(first: Double = 1, max: Double = 10) {
        self.first = first
        self.max = max
    }

    /// attempt counts from 1.
    public func delay(attempt: Int) -> Double {
        min(max, first * pow(2, Double(Swift.max(0, attempt - 1))))
    }
}

/// One WebSocket to a host's /ws, reconnecting like the web UI, with every message decoded.
public final class HostConnection: @unchecked Sendable {
    public let endpoint: HostEndpoint
    public let auth: HostAuth
    private let session: URLSession
    private let retry: RetryPolicy
    private let lock = NSLock()
    private var socket: URLSessionWebSocketTask?

    public init(endpoint: HostEndpoint, auth: HostAuth, session: URLSession = .shared, retry: RetryPolicy = RetryPolicy()) {
        self.endpoint = endpoint
        self.auth = auth
        self.session = session
        self.retry = retry
    }

    /// The upgrade request. Without a session the host lets in only WebSockets from its own origin (over
    /// Tailscale or on 127.0.0.1), so the app sends it; with a Linker Connect session it needs none.
    public static func request(endpoint: HostEndpoint, auth: HostAuth) -> URLRequest {
        var request = URLRequest(url: endpoint.webSocketURL)
        if auth == .none { request.setValue(endpoint.origin, forHTTPHeaderField: "Origin") }
        auth.authorize(&request)
        return request
    }

    /// Connects, and keeps reconnecting until the stream's consumer stops listening.
    public func events() -> AsyncStream<ConnectionEvent> {
        AsyncStream { continuation in
            let task = Task {
                await self.run(continuation)
                continuation.finish()
            }
            continuation.onTermination = { _ in task.cancel() }
        }
    }

    public func send(_ message: ClientMessage) async throws {
        let text = try message.json()
        guard let socket = current() else { throw URLError(.notConnectedToInternet) }
        try await socket.send(.string(text))
    }

    private func current() -> URLSessionWebSocketTask? {
        lock.lock()
        defer { lock.unlock() }
        return socket
    }

    private func setCurrent(_ task: URLSessionWebSocketTask?) {
        lock.lock()
        socket = task
        lock.unlock()
    }

    private func run(_ out: AsyncStream<ConnectionEvent>.Continuation) async {
        var attempt = 0
        while !Task.isCancelled {
            let task = session.webSocketTask(with: Self.request(endpoint: endpoint, auth: auth))
            // Previews and big snapshots: the default 1 MB limit is too small for a long chat.
            task.maximumMessageSize = 64 * 1024 * 1024
            setCurrent(task)
            task.resume()
            var opened = false
            do {
                while !Task.isCancelled {
                    let frame = try await withTaskCancellationHandler {
                        try await task.receive()
                    } onCancel: {
                        task.cancel(with: .goingAway, reason: nil)
                    }
                    if !opened {
                        opened = true
                        attempt = 0
                        out.yield(.connected)
                    }
                    let data: Data
                    switch frame {
                    case .string(let text): data = Data(text.utf8)
                    case .data(let bytes): data = bytes
                    @unknown default: continue
                    }
                    // A message this app can't read is skipped, not fatal.
                    if let message = try? ServerMessage.decode(data) { out.yield(.message(message)) }
                }
            } catch {
                let status = (task.response as? HTTPURLResponse)?.statusCode
                task.cancel(with: .goingAway, reason: nil)
                setCurrent(nil)
                if Task.isCancelled { return }
                if !opened, let status, status == 401 || status == 403 {
                    out.yield(.refused(status: status))
                    return
                }
                out.yield(.disconnected(reason: opened ? nil : error.localizedDescription))
            }
            attempt += 1
            let seconds = retry.delay(attempt: attempt)
            try? await Task.sleep(nanoseconds: UInt64(seconds * 1_000_000_000))
        }
        current()?.cancel(with: .goingAway, reason: nil)
        setCurrent(nil)
    }
}
