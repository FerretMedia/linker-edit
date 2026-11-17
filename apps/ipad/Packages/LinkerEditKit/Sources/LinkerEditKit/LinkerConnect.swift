import Foundation

// Linker Connect from the iPad: reaching an edit computer without Tailscale, signed in with Linker.
//
// 1. Sign in with Linker: Clerk OAuth with PKCE in ASWebAuthenticationSession, back to linkeredit://oauth/callback.
// 2. GET https://app.linker.cool/api/edit/computers with the Linker access token: your linked computers. The
//    Linker token only ever goes to one of them, at https://edit-<16 hex>.linker.cool.
// 3. POST https://<host>/api/connect/session with the Linker access token answers { code }; then
//    POST https://<host>/api/connect/native with { code } answers { session, expiresAt }.
// 4. Every request to the host, /ws included, sends Authorization: Bearer <session>. A 401 drops the session.
//
// Tokens and sessions are kept in the Keychain; sessions per host.

/// What Info.plist says about Linker's sign-in (LinkerOAuthClientID, LinkerOAuthIssuer). Empty until Linker
/// registers the iPad app at Clerk.
public struct LinkerOAuthConfig: Sendable, Equatable {
    public var clientID: String
    /// Clerk's Frontend API, such as https://clerk.linker.cool.
    public var issuer: URL?
    public var callbackScheme: String
    public var redirectURI: String
    public var scopes: String
    /// Where /api/edit/computers is.
    public var apiBase: URL

    public init(
        clientID: String,
        issuer: URL?,
        callbackScheme: String = "linkeredit",
        redirectURI: String = "linkeredit://oauth/callback",
        scopes: String = "openid profile email offline_access",
        apiBase: URL = URL(string: "https://app.linker.cool")!
    ) {
        self.clientID = clientID
        self.issuer = issuer
        self.callbackScheme = callbackScheme
        self.redirectURI = redirectURI
        self.scopes = scopes
        self.apiBase = apiBase
    }

    /// From the app's Info.plist; empty values leave sign-in off.
    public init(bundle: Bundle) {
        let clientID = (bundle.object(forInfoDictionaryKey: "LinkerOAuthClientID") as? String ?? "").trimmingCharacters(in: .whitespaces)
        let issuer = (bundle.object(forInfoDictionaryKey: "LinkerOAuthIssuer") as? String ?? "").trimmingCharacters(in: .whitespaces)
        self.init(clientID: clientID, issuer: issuer.isEmpty ? nil : URL(string: issuer))
    }

    public var isConfigured: Bool { !clientID.isEmpty && issuer != nil }
}

/// One of the edit computers linked to the Linker account, as Linker's /api/edit/computers lists it.
public struct LinkedComputer: Decodable, Hashable, Identifiable, Sendable {
    public var id: String
    public var name: String
    /// https://edit-….linker.cool
    public var url: String
    /// Whether its tunnel is connected now; nil when Linker couldn't tell.
    public var online: Bool?
    public var linkedAt: String?
    public var lastSeenAt: String?

    public init(from decoder: Decoder) throws {
        let c = try decoder.keyed()
        id = try c.required("id")
        name = c.value("name", "Edit computer")
        url = try c.required("url")
        online = c.optional("online")
        linkedAt = c.optional("linkedAt")
        lastSeenAt = c.optional("lastSeenAt")
    }

    public var endpoint: HostEndpoint? { HostEndpoint(string: url) }
}

/// A Linker Connect session on one host.
public struct HostSession: Codable, Equatable, Sendable {
    public var session: String
    public var expiresAt: Date

    public init(session: String, expiresAt: Date) {
        self.session = session
        self.expiresAt = expiresAt
    }
}

public enum ConnectError: Error, Equatable, LocalizedError {
    /// The build has no Linker client id or issuer yet.
    case notConfigured
    /// Sign in with Linker (again).
    case signInRequired
    case cancelled
    /// The host or Linker said no, in its own words.
    case refused(status: Int, message: String)
    case badResponse(String)
    /// Not one of the computers linked to the Linker account: the Linker sign-in isn't sent there.
    case notLinked(host: String)

    public var errorDescription: String? {
        switch self {
        case .notConfigured: return "Signing in with Linker isn't set up in this build yet."
        case .signInRequired: return "Sign in with Linker again."
        case .cancelled: return "Signing in was cancelled."
        case .refused(_, let message): return message
        case .badResponse(let message): return message
        case .notLinked(let host): return "\(host) isn't one of the computers linked to your Linker account."
        }
    }
}

/// Opens the sign-in page and answers the address it came back to: ASWebAuthenticationSession in the app.
public protocol WebAuthenticator: Sendable {
    func authenticate(url: URL, callbackScheme: String) async throws -> URL
}

/// The second way to reach a host, after typing a Tailscale address: Linker Connect.
public protocol ConnectAuth: AnyObject, Sendable {
    var isConfigured: Bool { get }
    var isSignedIn: Bool { get }
    func signIn() async throws
    func signOut()
    /// Your edit computers linked to Linker.
    func computers() async throws -> [LinkedComputer]
    /// A session on the host: the kept one while it lasts, else a new one.
    func session(for host: HostEndpoint) async throws -> HostSession
    /// Forgets the host's session, as after a 401.
    func dropSession(for host: HostEndpoint)
}

struct LinkerTokens: Codable, Equatable {
    var accessToken: String
    var refreshToken: String?
    var expiresAt: Date
}

public final class LinkerConnectClient: ConnectAuth, @unchecked Sendable {
    public let config: LinkerOAuthConfig
    private let deviceName: String
    private let store: SecretStore
    private let authenticator: WebAuthenticator
    private let http: URLSession
    private let now: @Sendable () -> Date

    static let tokensKey = "linker.tokens"
    static func sessionKey(_ host: HostEndpoint) -> String { "session:\(host.origin)" }
    /// Refresh this long before a token runs out.
    private let margin: TimeInterval = 60
    private let lock = NSLock()
    /// The origins of the linked computers Linker listed last: the only places the Linker token goes.
    private var linked: Set<String> = []

    /// Linker Connect addresses look like this; the Linker token goes nowhere else.
    public static func isLinkerComputer(_ host: HostEndpoint) -> Bool {
        host.origin.range(of: #"^https://edit-[0-9a-f]{16}\.linker\.cool$"#, options: .regularExpression) != nil
    }

    private func isListed(_ host: HostEndpoint) -> Bool {
        lock.lock()
        defer { lock.unlock() }
        return linked.contains(host.origin)
    }

    public init(
        config: LinkerOAuthConfig,
        deviceName: String,
        store: SecretStore,
        authenticator: WebAuthenticator,
        session: URLSession = .shared,
        now: @escaping @Sendable () -> Date = { Date() }
    ) {
        self.config = config
        self.deviceName = deviceName
        self.store = store
        self.authenticator = authenticator
        http = session
        self.now = now
    }

    public var isConfigured: Bool { config.isConfigured }
    public var isSignedIn: Bool { tokens != nil }

    private var tokens: LinkerTokens? {
        get { store.read(Self.tokensKey).flatMap { try? JSONDecoder().decode(LinkerTokens.self, from: $0) } }
        set {
            if let newValue, let data = try? JSONEncoder().encode(newValue) {
                store.write(data, for: Self.tokensKey)
            } else {
                store.delete(Self.tokensKey)
            }
        }
    }

    // MARK: Signing in with Linker

    struct ServerMetadata: Decodable {
        var issuer: String?
        var authorization_endpoint: URL
        var token_endpoint: URL
    }

    private func metadata() async throws -> ServerMetadata {
        guard config.isConfigured, let issuer = config.issuer else { throw ConnectError.notConfigured }
        let url = issuer.appending(path: ".well-known/oauth-authorization-server")
        let (data, response) = try await http.data(for: Self.jsonRequest(url))
        guard Self.status(response) == 200, let meta = try? JSONDecoder().decode(ServerMetadata.self, from: data) else {
            throw ConnectError.badResponse("Couldn't reach Linker's sign-in (\(Self.status(response))).")
        }
        return meta
    }

    public func signIn() async throws {
        let meta = try await metadata()
        let pkce = PKCE()
        let state = PKCE.randomString(bytes: 16)
        var components = URLComponents(url: meta.authorization_endpoint, resolvingAgainstBaseURL: false)!
        components.queryItems = (components.queryItems ?? []) + [
            URLQueryItem(name: "response_type", value: "code"),
            URLQueryItem(name: "client_id", value: config.clientID),
            URLQueryItem(name: "redirect_uri", value: config.redirectURI),
            URLQueryItem(name: "scope", value: config.scopes),
            URLQueryItem(name: "state", value: state),
            URLQueryItem(name: "code_challenge", value: pkce.challenge),
            URLQueryItem(name: "code_challenge_method", value: "S256"),
        ]
        let callback = try await authenticator.authenticate(url: components.url!, callbackScheme: config.callbackScheme)
        let query = URLComponents(url: callback, resolvingAgainstBaseURL: false)?.queryItems ?? []
        let field = { (name: String) in query.first { $0.name == name }?.value }
        if let error = field("error") {
            if error == "access_denied" { throw ConnectError.cancelled }
            throw ConnectError.refused(status: 400, message: field("error_description") ?? error)
        }
        guard field("state") == state else {
            throw ConnectError.badResponse("This sign-in isn't the one Linker-Edit started. Try again.")
        }
        if let iss = field("iss"), let expected = meta.issuer, iss != expected {
            throw ConnectError.badResponse("The sign-in came back from somewhere other than Linker.")
        }
        guard let code = field("code") else { throw ConnectError.badResponse("Linker sent no sign-in code.") }
        tokens = try await tokenRequest(meta, [
            "grant_type": "authorization_code",
            "code": code,
            "redirect_uri": config.redirectURI,
            "code_verifier": pkce.verifier,
            "client_id": config.clientID,
        ], refreshToken: nil)
    }

    public func signOut() {
        tokens = nil
    }

    struct TokenResponse: Decodable {
        var access_token: String?
        var refresh_token: String?
        var expires_in: Double?
        var error: String?
        var error_description: String?
    }

    private func tokenRequest(_ meta: ServerMetadata, _ fields: [String: String], refreshToken: String?) async throws -> LinkerTokens {
        var request = URLRequest(url: meta.token_endpoint)
        request.httpMethod = "POST"
        request.setValue("application/x-www-form-urlencoded", forHTTPHeaderField: "Content-Type")
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        request.httpBody = Data(Self.formEncoded(fields).utf8)
        let (data, response) = try await http.data(for: request)
        let body = try? JSONDecoder().decode(TokenResponse.self, from: data)
        guard Self.status(response) == 200, let access = body?.access_token else {
            if body?.error == "invalid_grant" {
                tokens = nil
                throw ConnectError.signInRequired
            }
            throw ConnectError.refused(
                status: Self.status(response),
                message: body?.error_description ?? "Linker answered \(Self.status(response))."
            )
        }
        return LinkerTokens(
            accessToken: access,
            refreshToken: body?.refresh_token ?? refreshToken,
            expiresAt: now().addingTimeInterval(body?.expires_in ?? 3600)
        )
    }

    /// A Linker access token that works for a while yet, refreshed when needed.
    func accessToken() async throws -> String {
        guard let current = tokens else { throw ConnectError.signInRequired }
        if current.expiresAt.timeIntervalSince(now()) > margin { return current.accessToken }
        guard let refresh = current.refreshToken else {
            tokens = nil
            throw ConnectError.signInRequired
        }
        let meta = try await metadata()
        let fresh = try await tokenRequest(meta, [
            "grant_type": "refresh_token",
            "refresh_token": refresh,
            "client_id": config.clientID,
        ], refreshToken: refresh)
        tokens = fresh
        return fresh.accessToken
    }

    // MARK: Computers and sessions

    struct ComputersResponse: Decodable {
        var computers: [Lossy<LinkedComputer>]
    }

    public func computers() async throws -> [LinkedComputer] {
        let token = try await accessToken()
        var request = Self.jsonRequest(config.apiBase.appending(path: "api/edit/computers"))
        request.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        let (data, response) = try await http.data(for: request)
        let status = Self.status(response)
        if status == 401 {
            tokens = nil
            throw ConnectError.signInRequired
        }
        guard status == 200, let body = try? JSONDecoder().decode(ComputersResponse.self, from: data) else {
            throw ConnectError.refused(status: status, message: Self.errorMessage(data) ?? "Linker answered \(status).")
        }
        let computers = body.computers.compactMap { $0.value }.filter { computer in
            computer.endpoint.map(Self.isLinkerComputer) ?? false
        }
        lock.lock()
        linked = Set(computers.compactMap { $0.endpoint?.origin })
        lock.unlock()
        return computers
    }

    struct CodeResponse: Decodable {
        var code: String
    }

    struct NativeSessionResponse: Decodable {
        var session: String
        var expiresAt: String
    }

    public func session(for host: HostEndpoint) async throws -> HostSession {
        guard Self.isLinkerComputer(host) else { throw ConnectError.notLinked(host: host.displayName) }
        if let data = store.read(Self.sessionKey(host)),
           let kept = try? JSONDecoder().decode(HostSession.self, from: data),
           kept.expiresAt.timeIntervalSince(now()) > margin {
            return kept
        }
        if !isListed(host) { _ = try await computers() }
        guard isListed(host) else { throw ConnectError.notLinked(host: host.displayName) }
        let token = try await accessToken()

        var ask = Self.jsonRequest(host.url(path: "/api/connect/session"), method: "POST")
        ask.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization")
        ask.httpBody = try JSONSerialization.data(withJSONObject: ["name": String(deviceName.prefix(64))])
        let (codeData, codeResponse) = try await http.data(for: ask)
        let codeStatus = Self.status(codeResponse)
        if codeStatus == 401 {
            // The host asked Linker, and Linker didn't take this sign-in.
            tokens = nil
            throw ConnectError.signInRequired
        }
        guard codeStatus == 200, let code = try? JSONDecoder().decode(CodeResponse.self, from: codeData) else {
            throw ConnectError.refused(status: codeStatus, message: Self.errorMessage(codeData) ?? "\(host.displayName) answered \(codeStatus).")
        }

        var swap = Self.jsonRequest(host.url(path: "/api/connect/native"), method: "POST")
        swap.httpBody = try JSONSerialization.data(withJSONObject: ["code": code.code])
        let (sessionData, sessionResponse) = try await http.data(for: swap)
        let sessionStatus = Self.status(sessionResponse)
        guard sessionStatus == 200,
              let body = try? JSONDecoder().decode(NativeSessionResponse.self, from: sessionData),
              let expiresAt = Self.date(body.expiresAt)
        else {
            throw ConnectError.refused(status: sessionStatus, message: Self.errorMessage(sessionData) ?? "\(host.displayName) answered \(sessionStatus).")
        }
        let session = HostSession(session: body.session, expiresAt: expiresAt)
        if let data = try? JSONEncoder().encode(session) { store.write(data, for: Self.sessionKey(host)) }
        return session
    }

    public func dropSession(for host: HostEndpoint) {
        store.delete(Self.sessionKey(host))
    }

    // MARK: Helpers

    static func jsonRequest(_ url: URL, method: String = "GET") -> URLRequest {
        var request = URLRequest(url: url)
        request.httpMethod = method
        request.setValue("application/json", forHTTPHeaderField: "Accept")
        if method != "GET" { request.setValue("application/json", forHTTPHeaderField: "Content-Type") }
        return request
    }

    static func status(_ response: URLResponse) -> Int {
        (response as? HTTPURLResponse)?.statusCode ?? 0
    }

    /// The { error } or { error, message } Linker-Edit and Linker answer with.
    static func errorMessage(_ data: Data) -> String? {
        guard let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return nil }
        return (object["message"] as? String) ?? (object["error"] as? String)
    }

    /// ISO 8601, with or without fractions of a second.
    static func date(_ text: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        formatter.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        if let date = formatter.date(from: text) { return date }
        formatter.formatOptions = [.withInternetDateTime]
        return formatter.date(from: text)
    }

    /// application/x-www-form-urlencoded, keys sorted.
    static func formEncoded(_ fields: [String: String]) -> String {
        let allowed = CharacterSet(charactersIn: "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~")
        func encode(_ text: String) -> String {
            text.addingPercentEncoding(withAllowedCharacters: allowed) ?? text
        }
        return fields.keys.sorted().map { "\(encode($0))=\(encode(fields[$0]!))" }.joined(separator: "&")
    }
}
