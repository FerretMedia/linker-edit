import CryptoKit
import Foundation

/// Proof Key for Code Exchange (RFC 7636), S256.
public struct PKCE: Sendable {
    public let verifier: String
    public let challenge: String

    public init(verifier: String = PKCE.randomString(bytes: 32)) {
        self.verifier = verifier
        challenge = PKCE.base64url(Data(SHA256.hash(data: Data(verifier.utf8))))
    }

    public static func base64url(_ data: Data) -> String {
        data.base64EncodedString()
            .replacingOccurrences(of: "+", with: "-")
            .replacingOccurrences(of: "/", with: "_")
            .replacingOccurrences(of: "=", with: "")
    }

    public static func randomString(bytes count: Int) -> String {
        var generator = SystemRandomNumberGenerator()
        let bytes = (0..<count).map { _ in UInt8.random(in: .min ... .max, using: &generator) }
        return base64url(Data(bytes))
    }
}
