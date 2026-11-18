import Foundation

/// A Linker-Edit host: the edit computer's address, such as https://studio.tail1234.ts.net:8443 over Tailscale or
/// https://edit-….linker.cool through Linker Connect.
public struct HostEndpoint: Codable, Hashable, Sendable {
    /// scheme://host[:port], with no path.
    public let baseURL: URL

    /// Reads what someone typed: "studio.tail1234.ts.net:8443" gets https://, a path is dropped.
    public init?(string: String) {
        var text = string.trimmingCharacters(in: .whitespacesAndNewlines)
        // URL parsing would take "my studio" and percent-encode it; no host has spaces.
        guard !text.isEmpty, !text.contains(where: { $0.isWhitespace }) else { return nil }
        if !text.contains("://") { text = "https://\(text)" }
        guard let parsed = URLComponents(string: text),
              let scheme = parsed.scheme?.lowercased(), scheme == "https" || scheme == "http",
              let host = parsed.host, !host.isEmpty
        else { return nil }
        var components = URLComponents()
        components.scheme = scheme
        components.host = host.lowercased()
        // As browsers write an origin: without the scheme's own port.
        let defaultPort = scheme == "https" ? 443 : 80
        components.port = parsed.port == defaultPort ? nil : parsed.port
        guard let url = components.url else { return nil }
        baseURL = url
    }

    /// The origin browsers would send from the host's own page. The host only lets in WebSockets that come
    /// with it (crates/server/src/http.rs), so the app sends it too.
    public var origin: String { baseURL.absoluteString }

    /// How the host is shown: its name without the scheme.
    public var displayName: String {
        guard let host = baseURL.host else { return origin }
        if let port = baseURL.port { return "\(host):\(port)" }
        return host
    }

    public var webSocketURL: URL {
        var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false)!
        components.scheme = baseURL.scheme == "https" ? "wss" : "ws"
        components.path = "/ws"
        return components.url!
    }

    /// A path on the host, such as the /preview/preview-1.mp4 the preview state names.
    public func url(path: String) -> URL {
        URL(string: path, relativeTo: baseURL)?.absoluteURL ?? baseURL.appendingPathComponent(path)
    }
}

/// How requests to a host prove who they're from.
public enum HostAuth: Hashable, Sendable {
    /// Over Tailscale Serve the host knows the device's owner; on 127.0.0.1 it's the same computer.
    case none
    /// A Linker Connect session, from POST /api/connect/native.
    case bearer(String)

    /// The headers every request to the host carries, also given to AVPlayer.
    public var headers: [String: String] {
        switch self {
        case .none: return [:]
        case .bearer(let session): return ["Authorization": "Bearer \(session)"]
        }
    }

    public func authorize(_ request: inout URLRequest) {
        for (name, value) in headers { request.setValue(value, forHTTPHeaderField: name) }
    }
}
