import Foundation
import XCTest
@testable import LinkerEditKit

/// Answers requests from a table instead of the network, and keeps what was asked.
final class MockURLProtocol: URLProtocol {
    struct Seen {
        let method: String
        let url: URL
        let headers: [String: String]
        let body: Data
    }

    nonisolated(unsafe) static var routes: [String: (status: Int, body: String)] = [:]
    nonisolated(unsafe) static var seen: [Seen] = []

    static func session() -> URLSession {
        let configuration = URLSessionConfiguration.ephemeral
        configuration.protocolClasses = [MockURLProtocol.self]
        return URLSession(configuration: configuration)
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }

    override func startLoading() {
        let url = request.url!
        let body = request.httpBody ?? request.httpBodyStream.map(Self.read) ?? Data()
        let method = request.httpMethod ?? "GET"
        Self.seen.append(Seen(method: method, url: url, headers: request.allHTTPHeaderFields ?? [:], body: body))
        let key = "\(method) \(url.absoluteString.components(separatedBy: "?")[0])"
        let route = Self.routes[key] ?? (404, #"{"error":"No route for \#(key)"}"#)
        let response = HTTPURLResponse(url: url, statusCode: route.status, httpVersion: "HTTP/1.1", headerFields: ["Content-Type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(route.body.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }

    override func stopLoading() {}

    private static func read(_ stream: InputStream) -> Data {
        stream.open()
        defer { stream.close() }
        var data = Data()
        var buffer = [UInt8](repeating: 0, count: 4096)
        while stream.hasBytesAvailable {
            let count = stream.read(&buffer, maxLength: buffer.count)
            if count <= 0 { break }
            data.append(buffer, count: count)
        }
        return data
    }
}

/// Stands in for ASWebAuthenticationSession: "signs in" by answering the callback the test wants.
final class FakeAuthenticator: WebAuthenticator, @unchecked Sendable {
    var opened: URL?
    var answer: (URL) -> URL

    init(answer: @escaping (URL) -> URL) {
        self.answer = answer
    }

    func authenticate(url: URL, callbackScheme: String) async throws -> URL {
        XCTAssertEqual(callbackScheme, "linkeredit")
        opened = url
        return answer(url)
    }
}

final class LinkerConnectTests: XCTestCase {
    private let issuer = URL(string: "https://clerk.linker.test")!
    private let host = HostEndpoint(string: "https://edit-0123456789abcdef.linker.cool")!
    private var store = MemorySecretStore()
    private var clock = Date(timeIntervalSince1970: 1_800_000_000)

    override func setUp() {
        super.setUp()
        MockURLProtocol.seen = []
        MockURLProtocol.routes = [
            "GET https://clerk.linker.test/.well-known/oauth-authorization-server": (200, """
            {"issuer":"https://clerk.linker.test","authorization_endpoint":"https://clerk.linker.test/oauth/authorize",
             "token_endpoint":"https://clerk.linker.test/oauth/token"}
            """),
            "POST https://clerk.linker.test/oauth/token": (200, #"{"access_token":"linker-1","refresh_token":"refresh-1","expires_in":3600}"#),
            "GET https://app.linker.cool/api/edit/computers": (200, #"{"limit":3,"computers":[{"id":"c1","name":"Studio","url":"https://edit-0123456789abcdef.linker.cool"}]}"#),
        ]
        store = MemorySecretStore()
    }

    private func client(
        clientID: String = "ipad-client",
        deviceName: String = "Jane's iPad",
        authenticator: FakeAuthenticator? = nil
    ) -> LinkerConnectClient {
        let fixed = clock
        return LinkerConnectClient(
            config: LinkerOAuthConfig(clientID: clientID, issuer: clientID.isEmpty ? nil : issuer),
            deviceName: deviceName,
            store: store,
            authenticator: authenticator ?? signedInAuthenticator(),
            session: MockURLProtocol.session(),
            now: { fixed }
        )
    }

    /// Comes back with the state it was sent, as Linker does.
    private func signedInAuthenticator(code: String = "auth-code") -> FakeAuthenticator {
        FakeAuthenticator { url in
            let state = URLComponents(url: url, resolvingAgainstBaseURL: false)!.queryItems!.first { $0.name == "state" }!.value!
            return URL(string: "linkeredit://oauth/callback?code=\(code)&state=\(state)&iss=https%3A%2F%2Fclerk.linker.test")!
        }
    }

    private func form(_ body: Data) -> [String: String] {
        var fields: [String: String] = [:]
        for pair in String(decoding: body, as: UTF8.self).split(separator: "&") {
            let parts = pair.split(separator: "=", maxSplits: 1).map { String($0).removingPercentEncoding ?? String($0) }
            fields[parts[0]] = parts.count > 1 ? parts[1] : ""
        }
        return fields
    }

    func testPKCEMatchesRFC7636() {
        let pkce = PKCE(verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")
        XCTAssertEqual(pkce.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM")
        XCTAssertEqual(PKCE().verifier.count, 43)
    }

    func testReadsItsConfigFromInfoPlistAndIsOffWithoutIt() async {
        XCTAssertFalse(LinkerOAuthConfig(clientID: "", issuer: nil).isConfigured)
        let linker = client(clientID: "")
        XCTAssertFalse(linker.isConfigured)
        do {
            try await linker.signIn()
            XCTFail("signed in without a client id")
        } catch {
            XCTAssertEqual(error as? ConnectError, .notConfigured)
        }
    }

    func testSignsInWithPKCEAndKeepsTheTokens() async throws {
        let authenticator = signedInAuthenticator()
        let linker = client(authenticator: authenticator)
        XCTAssertFalse(linker.isSignedIn)
        try await linker.signIn()
        XCTAssertTrue(linker.isSignedIn)

        let opened = try XCTUnwrap(authenticator.opened)
        XCTAssertEqual(opened.host, "clerk.linker.test")
        XCTAssertEqual(opened.path, "/oauth/authorize")
        let query = Dictionary(uniqueKeysWithValues: (URLComponents(url: opened, resolvingAgainstBaseURL: false)?.queryItems ?? []).map { ($0.name, $0.value ?? "") })
        XCTAssertEqual(query["response_type"], "code")
        XCTAssertEqual(query["client_id"], "ipad-client")
        XCTAssertEqual(query["redirect_uri"], "linkeredit://oauth/callback")
        XCTAssertEqual(query["scope"], "openid profile email offline_access")
        XCTAssertEqual(query["code_challenge_method"], "S256")

        let exchange = try XCTUnwrap(MockURLProtocol.seen.last { $0.url.path == "/oauth/token" })
        let fields = form(exchange.body)
        XCTAssertEqual(fields["grant_type"], "authorization_code")
        XCTAssertEqual(fields["code"], "auth-code")
        XCTAssertEqual(fields["redirect_uri"], "linkeredit://oauth/callback")
        XCTAssertEqual(fields["client_id"], "ipad-client")
        XCTAssertEqual(PKCE(verifier: try XCTUnwrap(fields["code_verifier"])).challenge, query["code_challenge"])
        XCTAssertEqual(store.keys, [LinkerConnectClient.tokensKey])
    }

    func testRefusesACallbackForAnotherSignIn() async {
        let linker = client(authenticator: FakeAuthenticator { _ in URL(string: "linkeredit://oauth/callback?code=x&state=forged")! })
        do {
            try await linker.signIn()
            XCTFail("took a forged state")
        } catch {
            XCTAssertFalse(linker.isSignedIn)
        }
        let cancelled = client(authenticator: FakeAuthenticator { _ in URL(string: "linkeredit://oauth/callback?error=access_denied")! })
        do {
            try await cancelled.signIn()
            XCTFail("signed in after a cancel")
        } catch {
            XCTAssertEqual(error as? ConnectError, .cancelled)
        }
    }

    func testListsTheLinkedComputers() async throws {
        MockURLProtocol.routes["GET https://app.linker.cool/api/edit/computers"] = (200, """
        {"limit":3,"computers":[
          {"id":"0b4e7a52-0000-4000-8000-000000000001","name":"Studio","url":"https://edit-0123456789abcdef.linker.cool","online":true,
           "linkedAt":"2026-11-14T10:00:00.000Z","lastSeenAt":null,"later":"field"},
          {"id":"0b4e7a52-0000-4000-8000-000000000002","name":"Laptop","url":"https://edit-fedcba9876543210.linker.cool","online":null,
           "linkedAt":"2026-11-15T10:00:00.000Z","lastSeenAt":"2026-11-16T10:00:00.000Z"},
          {"id":"0b4e7a52-0000-4000-8000-000000000003","name":"Elsewhere","url":"https://edit-0123456789abcdef.linker.cool.evil.example"},
          {"name":"broken"}]}
        """)
        let linker = client()
        try await linker.signIn()
        let computers = try await linker.computers()
        XCTAssertEqual(computers.map(\.name), ["Studio", "Laptop"])
        XCTAssertEqual(computers.map(\.online), [true, nil])
        XCTAssertEqual(computers.first?.endpoint, host)
        XCTAssertEqual(MockURLProtocol.seen.last?.headers["Authorization"], "Bearer linker-1")
    }

    func testSignsOutWhenLinkerRefusesTheToken() async throws {
        MockURLProtocol.routes["GET https://app.linker.cool/api/edit/computers"] = (401, #"{"error":"unauthorized"}"#)
        let linker = client()
        try await linker.signIn()
        do {
            _ = try await linker.computers()
            XCTFail("listed computers with a refused token")
        } catch {
            XCTAssertEqual(error as? ConnectError, .signInRequired)
        }
        XCTAssertFalse(linker.isSignedIn)
    }

    func testGetsASessionFromTheHostAndKeepsIt() async throws {
        MockURLProtocol.routes["POST https://edit-0123456789abcdef.linker.cool/api/connect/session"] = (200, #"{"code":"one-time"}"#)
        MockURLProtocol.routes["POST https://edit-0123456789abcdef.linker.cool/api/connect/native"] = (200, #"{"session":"opaque-session","expiresAt":"2027-01-15T10:00:00.000Z"}"#)
        let linker = client()
        try await linker.signIn()
        let session = try await linker.session(for: host)
        XCTAssertEqual(session.session, "opaque-session")
        XCTAssertEqual(session.expiresAt, LinkerConnectClient.date("2027-01-15T10:00:00Z"))

        let ask = try XCTUnwrap(MockURLProtocol.seen.first { $0.url.path == "/api/connect/session" })
        XCTAssertEqual(ask.method, "POST")
        XCTAssertEqual(ask.headers["Authorization"], "Bearer linker-1")
        XCTAssertEqual(try JSONSerialization.jsonObject(with: ask.body) as? [String: String], ["name": "Jane's iPad"])
        let swap = try XCTUnwrap(MockURLProtocol.seen.first { $0.url.path == "/api/connect/native" })
        XCTAssertEqual(swap.headers["Content-Type"], "application/json")
        XCTAssertNil(swap.headers["Authorization"])
        XCTAssertEqual(try JSONSerialization.jsonObject(with: swap.body) as? [String: String], ["code": "one-time"])

        // Kept per host: the next connection needs no requests.
        let requests = MockURLProtocol.seen.count
        let again = try await linker.session(for: host)
        XCTAssertEqual(again, session)
        XCTAssertEqual(MockURLProtocol.seen.count, requests)
        XCTAssertNotNil(store.read(LinkerConnectClient.sessionKey(host)))

        // A 401 from the host drops it.
        linker.dropSession(for: host)
        XCTAssertNil(store.read(LinkerConnectClient.sessionKey(host)))
    }

    func testSendsTheLinkerSignInOnlyToLinkedComputers() async throws {
        let linker = client()
        try await linker.signIn()
        let others = [
            "https://studio.tail1234.ts.net:8443",
            "http://edit-0123456789abcdef.linker.cool",
            "https://edit-0123456789abcdef.linker.cool.evil.example",
            // The right shape, but not one of this account's computers.
            "https://edit-1111111111111111.linker.cool",
        ]
        for address in others {
            let other = try XCTUnwrap(HostEndpoint(string: address))
            do {
                _ = try await linker.session(for: other)
                XCTFail("asked \(address) for a session")
            } catch {
                XCTAssertEqual(error as? ConnectError, .notLinked(host: other.displayName))
            }
        }
        XCTAssertFalse(MockURLProtocol.seen.contains { $0.url.path.hasPrefix("/api/connect") })
        XCTAssertTrue(LinkerConnectClient.isLinkerComputer(host))
    }

    func testSendsTheDeviceNameUpTo64Characters() async throws {
        MockURLProtocol.routes["POST https://edit-0123456789abcdef.linker.cool/api/connect/session"] = (200, #"{"code":"one-time"}"#)
        MockURLProtocol.routes["POST https://edit-0123456789abcdef.linker.cool/api/connect/native"] = (200, #"{"session":"s","expiresAt":"2027-01-15T10:00:00Z"}"#)
        let linker = client(deviceName: String(repeating: "i", count: 70))
        try await linker.signIn()
        _ = try await linker.session(for: host)
        let ask = try XCTUnwrap(MockURLProtocol.seen.first { $0.url.path == "/api/connect/session" })
        XCTAssertEqual(try JSONSerialization.jsonObject(with: ask.body) as? [String: String], ["name": String(repeating: "i", count: 64)])
    }

    func testPassesOnTheHostsRefusal() async throws {
        MockURLProtocol.routes["POST https://edit-0123456789abcdef.linker.cool/api/connect/session"] = (403, #"{"error":"Studio belongs to another Linker account."}"#)
        let linker = client()
        try await linker.signIn()
        do {
            _ = try await linker.session(for: host)
            XCTFail("got a session from a refusing host")
        } catch {
            XCTAssertEqual(error as? ConnectError, .refused(status: 403, message: "Studio belongs to another Linker account."))
        }
        XCTAssertTrue(linker.isSignedIn)
    }

    func testRefreshesAnExpiredToken() async throws {
        MockURLProtocol.routes["GET https://app.linker.cool/api/edit/computers"] = (200, #"{"limit":3,"computers":[]}"#)
        try await client().signIn()
        clock = clock.addingTimeInterval(3600)
        MockURLProtocol.routes["POST https://clerk.linker.test/oauth/token"] = (200, #"{"access_token":"linker-2","expires_in":3600}"#)
        let later = client()
        _ = try await later.computers()
        let refresh = try XCTUnwrap(MockURLProtocol.seen.last { $0.url.path == "/oauth/token" })
        XCTAssertEqual(form(refresh.body)["grant_type"], "refresh_token")
        XCTAssertEqual(form(refresh.body)["refresh_token"], "refresh-1")
        XCTAssertEqual(MockURLProtocol.seen.last?.headers["Authorization"], "Bearer linker-2")

        // Linker no longer takes the refresh token: signed out.
        clock = clock.addingTimeInterval(3600)
        MockURLProtocol.routes["POST https://clerk.linker.test/oauth/token"] = (400, #"{"error":"invalid_grant"}"#)
        do {
            _ = try await client().computers()
            XCTFail("listed computers without a sign-in")
        } catch {
            XCTAssertEqual(error as? ConnectError, .signInRequired)
        }
        XCTAssertFalse(client().isSignedIn)
    }
}
