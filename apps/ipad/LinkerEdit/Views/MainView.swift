import LinkerEditKit
import SwiftUI

/// The host's projects and threads beside the open thread's chat, with the preview in an inspector.
/// The split view, toolbar, inspector and popovers are the system's own, so they get Liquid Glass as they are.
struct MainView: View {
    @Environment(AppModel.self) private var model
    @State private var showPreview = false
    @State private var columns: NavigationSplitViewVisibility = .all

    var body: some View {
        NavigationSplitView(columnVisibility: $columns) {
            SidebarView()
                .navigationSplitViewColumnWidth(min: 260, ideal: 300, max: 380)
        } detail: {
            NavigationStack {
                ChatView()
                    .toolbar { toolbar }
            }
            .inspector(isPresented: $showPreview) {
                PreviewPanel()
                    .inspectorColumnWidth(min: 320, ideal: 420, max: 560)
            }
        }
        .overlay(alignment: .top) { ToastStack() }
    }

    @ToolbarContentBuilder
    private var toolbar: some ToolbarContent {
        // The toolbar puts the pill on glass of its own.
        ToolbarItem(placement: .topBarTrailing) {
            if let meta = model.store.state?.meta {
                ResolveStatusPill(status: meta.resolve) { model.send(.refreshResolve) }
            }
        }
        ToolbarItem(placement: .topBarTrailing) {
            Button {
                showPreview.toggle()
            } label: {
                Label("Preview", systemImage: "play.rectangle")
            }
            .accessibilityIdentifier("preview-toggle")
        }
    }
}

/// The server's toasts, floating at the top on glass, each for six seconds as on the desktop.
struct ToastStack: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(spacing: 8) {
            ForEach(model.store.toasts) { toast in
                HStack(alignment: .top, spacing: 8) {
                    Image(systemName: toast.tone == .error ? "exclamationmark.circle.fill" : "info.circle.fill")
                        .foregroundStyle(toast.tone == .error ? Color.red : Color.accentColor)
                    Text(toast.text).font(.callout)
                    Button {
                        model.store.dismiss(toast)
                    } label: {
                        Image(systemName: "xmark").font(.caption)
                    }
                    .buttonStyle(.borderless)
                    .foregroundStyle(.secondary)
                }
                .padding(.horizontal, 16)
                .padding(.vertical, 12)
                .glassEffect(.regular, in: .rect(cornerRadius: 18))
                .frame(maxWidth: 520)
                .task {
                    try? await Task.sleep(nanoseconds: 6_000_000_000)
                    model.store.dismiss(toast)
                }
            }
        }
        .padding(.top, 8)
        .animation(.default, value: model.store.toasts)
    }
}
