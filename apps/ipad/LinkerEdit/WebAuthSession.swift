import AuthenticationServices
import LinkerEditKit
import UIKit

/// Linker's sign-in page in ASWebAuthenticationSession, which hands back the linkeredit:// address it ends on.
final class WebAuthSession: WebAuthenticator, @unchecked Sendable {
    /// Kept while the sheet is up.
    private var current: ASWebAuthenticationSession?
    private var anchor: WindowAnchor?

    func authenticate(url: URL, callbackScheme: String) async throws -> URL {
        try await withCheckedThrowingContinuation { (continuation: CheckedContinuation<URL, Error>) in
            Task { @MainActor in
                self.start(url: url, callbackScheme: callbackScheme, continuation: continuation)
            }
        }
    }

    @MainActor
    private func start(url: URL, callbackScheme: String, continuation: CheckedContinuation<URL, Error>) {
        var finished = false
        let finish = { (result: Result<URL, Error>) in
            guard !finished else { return }
            finished = true
            self.current = nil
            self.anchor = nil
            continuation.resume(with: result)
        }
        let session = ASWebAuthenticationSession(url: url, callbackURLScheme: callbackScheme) { callback, error in
            if let callback {
                finish(.success(callback))
            } else if let error = error as? ASWebAuthenticationSessionError, error.code == .canceledLogin {
                finish(.failure(ConnectError.cancelled))
            } else {
                finish(.failure(error ?? ConnectError.cancelled))
            }
        }
        let anchor = WindowAnchor()
        session.presentationContextProvider = anchor
        current = session
        self.anchor = anchor
        if !session.start() {
            finish(.failure(ConnectError.badResponse("Couldn't open Linker's sign-in.")))
        }
    }
}

/// The window the sign-in sheet shows over.
final class WindowAnchor: NSObject, ASWebAuthenticationPresentationContextProviding {
    func presentationAnchor(for session: ASWebAuthenticationSession) -> ASPresentationAnchor {
        MainActor.assumeIsolated {
            let windows = UIApplication.shared.connectedScenes.compactMap { $0 as? UIWindowScene }.flatMap(\.windows)
            return windows.first(where: \.isKeyWindow) ?? windows.first ?? UIWindow()
        }
    }
}
