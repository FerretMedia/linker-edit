import LinkerEditKit
import SwiftUI

@main
@MainActor
struct LinkerEditApp: App {
    @State private var model = AppModel.launch()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environment(model)
                .preferredColorScheme(.dark)
                .tint(Theme.accent)
        }
    }
}

/// The connect screen until a host is chosen, then the host's projects and chat.
struct RootView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        Group {
            if model.endpoint == nil {
                ConnectView()
            } else {
                MainView()
            }
        }
        .background(Theme.background)
    }
}

/// Loosely the desktop app's dark theme: black, neutral greys, a blue accent.
enum Theme {
    static let background = Color.black
    static let card = Color(white: 0.07)
    static let raised = Color(white: 0.11)
    static let border = Color.white.opacity(0.08)
    static let muted = Color(white: 0.62)
    static let accent = Color(red: 0.29, green: 0.40, blue: 0.95)
    static let success = Color(red: 0.25, green: 0.78, blue: 0.47)
    static let warning = Color(red: 0.96, green: 0.68, blue: 0.20)
    static let danger = Color(red: 0.94, green: 0.33, blue: 0.33)
}
