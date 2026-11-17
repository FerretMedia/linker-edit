import Foundation
import LinkerEditKit
import Observation
import UIKit

/// The host this iPad is connected to and how: a typed address (Tailscale) or Linker Connect.
@MainActor
@Observable
final class AppModel {
    let store: AppStore
    let linker: LinkerConnectClient
    private let recentHosts = RecentHosts()

    /// The host being shown; nil on the connect screen.
    private(set) var endpoint: HostEndpoint?
    private(set) var auth: HostAuth = .none
    private(set) var via: RecentHost.Via = .direct
    /// The WebSocket is open now.
    private(set) var online = false
    /// Why the last connection failed, for the connect screen or the banner.
    var problem: String?
    private(set) var recent: [RecentHost] = []
    /// Shows a fixed chat instead of a host, for screenshots (launch argument -demo).
    let demo: Bool

    private var connection: HostConnection?
    private var listening: Task<Void, Never>?

    init(demo: Bool = false) {
        self.demo = demo
        store = AppStore()
        linker = LinkerConnectClient(
            config: LinkerOAuthConfig(bundle: .main),
            deviceName: UIDevice.current.name,
            store: KeychainStore(),
            authenticator: WebAuthSession()
        )
        recent = recentHosts.all
    }

    /// How the app was launched: -demo shows the fixed chat, -host <url> connects at once (UI tests),
    /// -reset forgets recent hosts and the typed address.
    static func launch(arguments: [String] = ProcessInfo.processInfo.arguments) -> AppModel {
        if arguments.contains("-reset") {
            RecentHosts().all.forEach { RecentHosts().forget($0) }
            UserDefaults.standard.removeObject(forKey: "lastTypedHost")
        }
        if arguments.contains("-demo") {
            let model = AppModel(demo: true)
            model.endpoint = HostEndpoint(string: "https://studio.tail1234.ts.net:8443")
            model.online = true
            model.store.apply(DemoData.snapshot())
            return model
        }
        let model = AppModel()
        if let index = arguments.firstIndex(of: "-host"), index + 1 < arguments.count,
           let endpoint = HostEndpoint(string: arguments[index + 1]) {
            model.connect(to: endpoint, auth: .none, via: .direct, name: nil)
        }
        return model
    }

    // MARK: Connecting

    func connect(to endpoint: HostEndpoint, auth: HostAuth, via: RecentHost.Via, name: String?) {
        disconnect()
        self.endpoint = endpoint
        self.auth = auth
        self.via = via
        problem = nil
        recentHosts.remember(RecentHost(endpoint: endpoint, name: name, via: via, lastUsed: Date()))
        recent = recentHosts.all

        let connection = HostConnection(endpoint: endpoint, auth: auth)
        self.connection = connection
        listening = Task { [weak self] in
            for await event in connection.events() {
                guard let self else { return }
                self.handle(event)
            }
        }
    }

    /// Typed in on the connect screen. A host reached through Linker before gets its kept session.
    func connect(typed text: String) {
        guard let endpoint = HostEndpoint(string: text) else {
            problem = "That isn't a web address. Try https://your-computer.your-tailnet.ts.net:8443."
            return
        }
        connect(to: endpoint, auth: .none, via: .direct, name: nil)
    }

    func open(_ host: RecentHost) async {
        switch host.via {
        case .direct: connect(to: host.endpoint, auth: .none, via: .direct, name: host.name)
        case .linker: await openWithLinker(host.endpoint, name: host.name)
        }
    }

    /// Linker Connect: a session on the computer from the Linker sign-in, then the WebSocket with it.
    func openWithLinker(_ endpoint: HostEndpoint, name: String?) async {
        problem = nil
        do {
            if !linker.isSignedIn { try await linker.signIn() }
            let session = try await linker.session(for: endpoint)
            connect(to: endpoint, auth: .bearer(session.session), via: .linker, name: name)
        } catch ConnectError.cancelled {
            return
        } catch {
            problem = error.localizedDescription
        }
    }

    func forget(_ host: RecentHost) {
        recentHosts.forget(host)
        if host.via == .linker { linker.dropSession(for: host.endpoint) }
        recent = recentHosts.all
    }

    /// Back to the connect screen.
    func disconnect() {
        listening?.cancel()
        listening = nil
        connection = nil
        endpoint = nil
        online = false
        if !demo { store.reset() }
    }

    private func handle(_ event: ConnectionEvent) {
        switch event {
        case .connected:
            online = true
            problem = nil
        case .message(let message):
            store.apply(message)
        case .disconnected(let reason):
            online = false
            if store.state == nil, let reason { problem = reason }
        case .refused(let status):
            let host = endpoint
            disconnect()
            if via == .linker, let host {
                // The session ran out or this iPad was signed out on the computer: sign in again.
                linker.dropSession(for: host)
                problem = "\(host.displayName) signed this iPad out (\(status)). Open it again to sign in."
            } else {
                problem = "The computer turned this iPad away (\(status)). Is remote access on there, "
                    + "and is this iPad logged in to Tailscale with the computer owner's account?"
            }
        }
    }

    // MARK: Talking to the host

    func send(_ message: ClientMessage) {
        guard let connection else { return }
        Task {
            do {
                try await connection.send(message)
            } catch {
                self.problem = "Not sent: the connection to the computer is down. It reconnects by itself."
            }
        }
    }

    /// A file on the host, such as a preview, with what AVPlayer needs to send for it.
    func hostURL(_ path: String) -> (url: URL, headers: [String: String])? {
        guard let endpoint else { return nil }
        return (endpoint.url(path: path), auth.headers)
    }
}
