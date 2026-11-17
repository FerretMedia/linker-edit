import Foundation

/// A host this iPad connected to, for the connect screen.
public struct RecentHost: Codable, Hashable, Identifiable, Sendable {
    public enum Via: String, Codable, Sendable {
        /// Typed in: Tailscale, or the same network.
        case direct
        /// Through Linker Connect, with a session from signing in with Linker.
        case linker
    }

    public var endpoint: HostEndpoint
    /// The computer's name, once known.
    public var name: String?
    public var via: Via
    public var lastUsed: Date

    public var id: String { endpoint.origin }

    public init(endpoint: HostEndpoint, name: String?, via: Via, lastUsed: Date) {
        self.endpoint = endpoint
        self.name = name
        self.via = via
        self.lastUsed = lastUsed
    }
}

/// The hosts this iPad connected to, newest first, kept in UserDefaults.
public struct RecentHosts {
    public static let limit = 8
    private let defaults: UserDefaults
    private let key: String

    public init(defaults: UserDefaults = .standard, key: String = "recentHosts") {
        self.defaults = defaults
        self.key = key
    }

    public var all: [RecentHost] {
        guard let data = defaults.data(forKey: key),
              let hosts = try? JSONDecoder().decode([RecentHost].self, from: data)
        else { return [] }
        return hosts
    }

    /// Puts the host first; a name it had stays when none is given.
    public func remember(_ host: RecentHost) {
        let earlier = all.first { $0.id == host.id }
        var entry = host
        if entry.name == nil { entry.name = earlier?.name }
        save([entry] + all.filter { $0.id != host.id })
    }

    public func forget(_ host: RecentHost) {
        save(all.filter { $0.id != host.id })
    }

    private func save(_ hosts: [RecentHost]) {
        let kept = Array(hosts.prefix(Self.limit))
        if let data = try? JSONEncoder().encode(kept) { defaults.set(data, forKey: key) }
    }
}
