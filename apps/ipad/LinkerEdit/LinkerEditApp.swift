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
                .preferredColorScheme(Self.testAppearance)
        }
    }

    /// The system's light or dark mode decides, except in the UI tests, which pass -uiTestAppearance light|dark
    /// to take the screenshots in both.
    private static let testAppearance: ColorScheme? = {
        let arguments = ProcessInfo.processInfo.arguments
        guard let index = arguments.firstIndex(of: "-uiTestAppearance"), index + 1 < arguments.count else { return nil }
        switch arguments[index + 1] {
        case "dark": return .dark
        case "light": return .light
        default: return nil
        }
    }()
}

/// The connect screen until a host is chosen, then the host's projects and chat.
struct RootView: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        if model.endpoint == nil {
            ConnectView()
        } else {
            MainView()
        }
    }
}
