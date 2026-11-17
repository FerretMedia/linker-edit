import AVKit
import LinkerEditKit
import SwiftUI

/// A small render of the timeline open in Resolve, made on the computer and played here from /preview/….
struct PreviewPanel: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Preview").font(.headline)
            switch model.store.state?.meta.preview ?? .idle {
            case .idle:
                Text("Renders the timeline open in Resolve as a small video, color and effects included.")
                    .font(.callout)
                    .foregroundStyle(Theme.muted)
                renderButton("Render preview")
            case .rendering(let timeline, let percent):
                Text(timeline.map { "Rendering \($0.name)…" } ?? "Waiting for Resolve…")
                    .font(.callout)
                ProgressView(value: min(max(percent, 0), 100), total: 100)
                Button("Stop", role: .destructive) { model.send(.stopPreview) }
                    .buttonStyle(.bordered)
            case .ready(let timeline, let url, _, let outdated):
                if let file = model.hostURL(url) {
                    PreviewPlayer(url: file.url, headers: file.headers)
                        .aspectRatio(16 / 9, contentMode: .fit)
                        .clipShape(RoundedRectangle(cornerRadius: 10))
                }
                if let timeline { Text(timeline.name).font(.callout) }
                if outdated {
                    Label("Resolve changed since this render.", systemImage: "clock.arrow.circlepath")
                        .font(.footnote)
                        .foregroundStyle(Theme.warning)
                }
                renderButton(outdated ? "Render again" : "Render the open timeline")
            case .error(let message):
                Label(message, systemImage: "exclamationmark.triangle")
                    .font(.callout)
                    .foregroundStyle(Theme.danger)
                renderButton("Try again")
            case .unknown:
                Text("This computer's Linker-Edit is newer than this app.").foregroundStyle(Theme.muted)
            }
            Spacer()
        }
        .padding(20)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(Theme.card)
    }

    private func renderButton(_ title: String) -> some View {
        Button {
            model.send(.renderPreview(timelineId: nil))
        } label: {
            Label(title, systemImage: "play.rectangle")
        }
        .buttonStyle(.bordered)
        .disabled(!model.online)
    }
}

/// AVPlayer on the host's file. Seeking works because the host answers Range requests; through Linker Connect
/// every request carries the session.
private struct PreviewPlayer: View {
    let url: URL
    let headers: [String: String]
    @State private var player: AVPlayer?

    var body: some View {
        VideoPlayer(player: player)
            .background(Color.black)
            .task(id: url) {
                let options: [String: Any] = headers.isEmpty ? [:] : ["AVURLAssetHTTPHeaderFieldsKey": headers]
                let asset = AVURLAsset(url: url, options: options)
                player = AVPlayer(playerItem: AVPlayerItem(asset: asset))
            }
            .onDisappear { player?.pause() }
    }
}

/// Whether Resolve is reachable on the computer, and what it has open. Tapping it checks again.
struct ResolveStatusPill: View {
    let status: ResolveStatus
    let refresh: () -> Void
    @State private var showHint = false

    private var look: (color: Color, text: String, hint: String?) {
        switch status {
        case .checking: return (Theme.muted, "Checking Resolve…", nil)
        case .connected(_, _, let project, let timeline, _):
            let open = [project, timeline].compactMap { $0 }.joined(separator: " · ")
            return (Theme.success, open.isEmpty ? "Resolve" : open, nil)
        case .notRunning: return (Theme.danger, "Resolve isn't running", "Start DaVinci Resolve Studio on the computer.")
        case .unreachable(let hint): return (Theme.warning, "Can't reach Resolve", hint)
        case .blocked(let hint): return (Theme.warning, "Resolve is blocked", hint)
        case .setupNeeded(let hint): return (Theme.warning, "Setup needed", hint)
        case .unknown(let state): return (Theme.muted, "Resolve: \(state)", nil)
        }
    }

    var body: some View {
        Button {
            if look.hint != nil { showHint = true }
            refresh()
        } label: {
            HStack(spacing: 6) {
                Circle().fill(look.color).frame(width: 8, height: 8)
                Text(look.text).font(.footnote).lineLimit(1)
            }
            .padding(.horizontal, 10)
            .padding(.vertical, 5)
            .background(Theme.raised, in: Capsule())
        }
        .buttonStyle(.plain)
        .accessibilityIdentifier("resolve-status")
        .popover(isPresented: $showHint) {
            Text(look.hint ?? "")
                .font(.callout)
                .padding()
                .frame(maxWidth: 360)
        }
    }
}
