import Foundation
import XCTest
@testable import LinkerEditKit

final class ConnectionTests: XCTestCase {
    func testReadsTypedHosts() throws {
        let tailscale = try XCTUnwrap(HostEndpoint(string: " studio.tail1234.ts.net:8443/some/path "))
        XCTAssertEqual(tailscale.origin, "https://studio.tail1234.ts.net:8443")
        XCTAssertEqual(tailscale.webSocketURL.absoluteString, "wss://studio.tail1234.ts.net:8443/ws")
        XCTAssertEqual(tailscale.displayName, "studio.tail1234.ts.net:8443")
        XCTAssertEqual(tailscale.url(path: "/preview/preview-1.mp4").absoluteString, "https://studio.tail1234.ts.net:8443/preview/preview-1.mp4")

        let local = try XCTUnwrap(HostEndpoint(string: "http://127.0.0.1:4317/"))
        XCTAssertEqual(local.origin, "http://127.0.0.1:4317")
        XCTAssertEqual(local.webSocketURL.absoluteString, "ws://127.0.0.1:4317/ws")

        // Browsers leave the scheme's own port out of an origin, and the host compares origins as text.
        XCTAssertEqual(HostEndpoint(string: "https://Edit-AB12.linker.cool:443")?.origin, "https://edit-ab12.linker.cool")

        XCTAssertNil(HostEndpoint(string: ""))
        XCTAssertNil(HostEndpoint(string: "my studio"))
        XCTAssertNil(HostEndpoint(string: "ftp://studio"))
        XCTAssertNil(HostEndpoint(string: "https://"))
    }

    func testTheUpgradeCarriesTheOriginAndTheSession() throws {
        let host = try XCTUnwrap(HostEndpoint(string: "https://edit-ab12.linker.cool"))
        let tailscale = HostConnection.request(endpoint: host, auth: .none)
        XCTAssertEqual(tailscale.url?.absoluteString, "wss://edit-ab12.linker.cool/ws")
        XCTAssertEqual(tailscale.value(forHTTPHeaderField: "Origin"), "https://edit-ab12.linker.cool")
        XCTAssertNil(tailscale.value(forHTTPHeaderField: "Authorization"))

        let linker = HostConnection.request(endpoint: host, auth: .bearer("s3cret"))
        XCTAssertEqual(linker.value(forHTTPHeaderField: "Authorization"), "Bearer s3cret")
        XCTAssertEqual(HostAuth.bearer("s3cret").headers, ["Authorization": "Bearer s3cret"])
    }

    func testBacksOffUpToTenSeconds() {
        let retry = RetryPolicy()
        XCTAssertEqual((1...6).map { retry.delay(attempt: $0) }, [1, 2, 4, 8, 10, 10])
    }

    func testRemembersRecentHostsNewestFirst() throws {
        let defaults = try XCTUnwrap(UserDefaults(suiteName: "RecentHostsTests"))
        defaults.removePersistentDomain(forName: "RecentHostsTests")
        let recent = RecentHosts(defaults: defaults)
        let studio = try XCTUnwrap(HostEndpoint(string: "studio.tail1234.ts.net:8443"))
        let laptop = try XCTUnwrap(HostEndpoint(string: "laptop.tail1234.ts.net:8443"))
        recent.remember(RecentHost(endpoint: studio, name: "Studio", via: .direct, lastUsed: Date(timeIntervalSince1970: 1)))
        recent.remember(RecentHost(endpoint: laptop, name: nil, via: .direct, lastUsed: Date(timeIntervalSince1970: 2)))
        recent.remember(RecentHost(endpoint: studio, name: nil, via: .linker, lastUsed: Date(timeIntervalSince1970: 3)))
        XCTAssertEqual(recent.all.map(\.endpoint), [studio, laptop])
        XCTAssertEqual(recent.all.first?.name, "Studio")
        XCTAssertEqual(recent.all.first?.via, .linker)
        for index in 0..<10 {
            recent.remember(RecentHost(endpoint: HostEndpoint(string: "pc\(index).example")!, name: nil, via: .direct, lastUsed: Date()))
        }
        XCTAssertEqual(recent.all.count, RecentHosts.limit)
        recent.forget(recent.all[0])
        XCTAssertEqual(recent.all.count, RecentHosts.limit - 1)
    }

    /// Against a real Linker-Edit server, when CI started one (LINKER_EDIT_URL, see .github/workflows/ipad.yml).
    func testConnectsToARealServer() async throws {
        guard let address = ProcessInfo.processInfo.environment["LINKER_EDIT_URL"], let host = HostEndpoint(string: address) else {
            throw XCTSkip("Set LINKER_EDIT_URL to a running Linker-Edit server.")
        }
        let connection = HostConnection(endpoint: host, auth: .none)
        var connected = false
        for await event in connection.events() {
            switch event {
            case .connected:
                connected = true
            case .message(let message):
                guard case .snapshot(let state) = message else { continue }
                XCTAssertTrue(connected)
                XCTAssertFalse(state.meta.version.isEmpty)
                try await connection.send(.refreshResolve)
                return
            case .disconnected(let reason):
                return XCTFail("Disconnected: \(reason ?? "")")
            case .refused(let status):
                return XCTFail("Refused: \(status)")
            }
        }
    }
}
