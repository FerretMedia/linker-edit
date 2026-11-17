import LinkerEditKit
import SwiftUI

/// Writing to the agent, stopping it, and choosing between full access and supervised: a glass bar floating over
/// the chat, with the send button beside it that turns into Stop while the agent works.
struct ComposerView: View {
    @Environment(AppModel.self) private var model
    @State private var text = ""
    @FocusState private var focused: Bool
    @Namespace private var glass

    private var meta: AppMeta? { model.store.state?.meta }
    private var busy: Bool { meta?.busy == true }
    private var canSend: Bool {
        model.online && meta?.threadId != nil && !busy && !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let blocked = meta?.blocked {
                Label(blocked.message, systemImage: "exclamationmark.triangle")
                    .font(.footnote)
                    .foregroundStyle(.orange)
                    .padding(.horizontal, 12)
                    .padding(.vertical, 6)
                    .glassEffect(.regular, in: .capsule)
            }
            GlassEffectContainer(spacing: 10) {
                HStack(alignment: .bottom, spacing: 10) {
                    field
                    action
                }
            }
        }
        .padding(.horizontal, 20)
        .padding(.bottom, 12)
        .padding(.top, 6)
        .frame(maxWidth: 860)
        .frame(maxWidth: .infinity)
        .animation(.smooth(duration: 0.25), value: busy)
        .onChange(of: model.store.refused) {
            // The host didn't pass the message on: it goes back here, as on the desktop.
            guard let refused = model.store.refused, refused.threadId == meta?.threadId else { return }
            text = refused.text
            model.store.refused = nil
        }
    }

    private var field: some View {
        VStack(alignment: .leading, spacing: 10) {
            TextField("Ask for an edit…", text: $text, axis: .vertical)
                .lineLimit(1...8)
                .focused($focused)
                .onSubmit(send)
                .accessibilityIdentifier("composer")
            if let meta {
                HStack(spacing: 12) {
                    Menu {
                        Picker("Access", selection: Binding(
                            get: { meta.approvalMode },
                            set: { model.send(.setApprovalMode($0)) }
                        )) {
                            Text("Full access").tag(ApprovalMode.auto)
                            Text("Supervised").tag(ApprovalMode.ask)
                        }
                    } label: {
                        Label(
                            meta.approvalMode == .ask ? "Supervised" : "Full access",
                            systemImage: meta.approvalMode == .ask ? "hand.raised" : "bolt"
                        )
                        .font(.footnote)
                    }
                    Text(meta.provider.label).font(.footnote).foregroundStyle(.secondary)
                    Spacer()
                }
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 12)
        .glassEffect(.regular.interactive(), in: .rect(cornerRadius: 24))
    }

    /// Send, or Stop while the agent works; one glass circle that morphs between the two.
    @ViewBuilder
    private var action: some View {
        if busy {
            Button {
                model.send(.stop)
            } label: {
                Image(systemName: "stop.fill")
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(.white)
                    .frame(width: 48, height: 48)
            }
            .buttonStyle(.plain)
            .glassEffect(.regular.tint(.red).interactive(), in: .circle)
            .glassEffectID("action", in: glass)
            .accessibilityLabel("Stop")
            .accessibilityIdentifier("stop")
        } else {
            Button(action: send) {
                Image(systemName: "arrow.up")
                    .font(.title3.weight(.semibold))
                    .foregroundStyle(canSend ? Color.white : Color.secondary)
                    .frame(width: 48, height: 48)
            }
            .buttonStyle(.plain)
            .glassEffect(canSend ? Glass.regular.tint(Color.accentColor).interactive() : Glass.regular, in: .circle)
            .glassEffectID("action", in: glass)
            .disabled(!canSend)
            .accessibilityLabel("Send")
            .accessibilityIdentifier("send")
        }
    }

    private func send() {
        guard canSend else { return }
        model.send(.send(text: text.trimmingCharacters(in: .whitespacesAndNewlines)))
        text = ""
    }
}
