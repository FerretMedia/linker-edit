import LinkerEditKit
import SwiftUI

/// Writing to the agent, stopping it, and choosing between full access and supervised.
struct ComposerView: View {
    @Environment(AppModel.self) private var model
    @State private var text = ""
    @FocusState private var focused: Bool

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
                    .foregroundStyle(Theme.warning)
            }
            VStack(alignment: .leading, spacing: 10) {
                TextField("Ask for an edit…", text: $text, axis: .vertical)
                    .lineLimit(1...8)
                    .focused($focused)
                    .onSubmit(send)
                    .accessibilityIdentifier("composer")
                HStack(spacing: 12) {
                    if let meta {
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
                        Text(meta.provider.label).font(.footnote).foregroundStyle(Theme.muted)
                    }
                    Spacer()
                    if busy {
                        Button {
                            model.send(.stop)
                        } label: {
                            Image(systemName: "stop.circle.fill").font(.title2)
                        }
                        .accessibilityLabel("Stop")
                        .accessibilityIdentifier("stop")
                    } else {
                        Button(action: send) {
                            Image(systemName: "arrow.up.circle.fill").font(.title2)
                        }
                        .disabled(!canSend)
                        .accessibilityLabel("Send")
                        .accessibilityIdentifier("send")
                    }
                }
            }
            .padding(12)
            .background(Theme.card, in: RoundedRectangle(cornerRadius: 18))
            .overlay(RoundedRectangle(cornerRadius: 18).strokeBorder(Theme.border))
        }
        .padding(.horizontal, 20)
        .padding(.bottom, 12)
        .padding(.top, 6)
        .frame(maxWidth: 860)
        .frame(maxWidth: .infinity)
        .onChange(of: model.store.refused) {
            // The host didn't pass the message on: it goes back here, as on the desktop.
            guard let refused = model.store.refused, refused.threadId == meta?.threadId else { return }
            text = refused.text
            model.store.refused = nil
        }
    }

    private func send() {
        guard canSend else { return }
        model.send(.send(text: text.trimmingCharacters(in: .whitespacesAndNewlines)))
        text = ""
    }
}
