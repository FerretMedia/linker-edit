import LinkerEditKit
import SwiftUI

/// Projects with their threads, as the desktop sidebar lists them; picking one opens it on the host too.
struct SidebarView: View {
    @Environment(AppModel.self) private var model

    private var selection: Binding<String?> {
        Binding(
            get: { model.store.state?.meta.threadId },
            set: { threadId in
                guard let threadId, threadId != model.store.state?.meta.threadId else { return }
                model.send(.selectThread(threadId: threadId))
            }
        )
    }

    var body: some View {
        List(selection: selection) {
            if let meta = model.store.state?.meta {
                if meta.projects.isEmpty {
                    Text("No projects yet. Add one in Linker-Edit on the computer.")
                        .font(.callout)
                        .foregroundStyle(Theme.muted)
                }
                ForEach(meta.projects.filter { !$0.archived || $0.threads.contains { !$0.archived } }) { project in
                    Section {
                        ForEach(project.threads.filter { !$0.archived && $0.settledAt == nil }) { thread in
                            ThreadRow(thread: thread).tag(thread.id)
                        }
                    } header: {
                        ProjectHeader(project: project) { model.send(.newThread(projectId: project.id)) }
                    }
                }
            } else {
                HStack(spacing: 8) {
                    ProgressView().controlSize(.small)
                    Text("Connecting…").foregroundStyle(Theme.muted)
                }
            }
        }
        .listStyle(.sidebar)
        .scrollContentBackground(.hidden)
        .background(Theme.card)
        .navigationTitle(model.store.state?.meta.computer.nilIfEmpty ?? model.endpoint?.displayName ?? "Linker-Edit")
        .safeAreaInset(edge: .bottom) { HostFooter() }
    }
}

private struct ProjectHeader: View {
    let project: ProjectSummary
    let newThread: () -> Void

    var body: some View {
        HStack(spacing: 8) {
            Text(project.name.isEmpty ? "Untitled" : project.name)
                .font(.subheadline.weight(.semibold))
                .foregroundStyle(.primary)
                .textCase(nil)
            if project.open {
                Text("Open in Resolve")
                    .font(.caption2)
                    .foregroundStyle(Theme.success)
                    .textCase(nil)
            }
            Spacer()
            Button(action: newThread) {
                Image(systemName: "square.and.pencil")
            }
            .buttonStyle(.borderless)
            .accessibilityLabel("New thread in \(project.name)")
        }
    }
}

private struct ThreadRow: View {
    let thread: ThreadSummary

    var body: some View {
        HStack(spacing: 8) {
            Text(thread.title)
                .lineLimit(1)
                .fontWeight(thread.unread ? .semibold : .regular)
            Spacer(minLength: 4)
            if thread.waiting {
                Image(systemName: "hand.raised.fill")
                    .font(.caption)
                    .foregroundStyle(Theme.warning)
                    .accessibilityLabel("Waiting for you")
            } else if thread.busy {
                ProgressView().controlSize(.mini)
            } else if thread.unread {
                Circle().fill(Theme.accent).frame(width: 7, height: 7).accessibilityLabel("Unread")
            }
        }
    }
}

/// Which computer this is, whether it's reachable, and the way back to the connect screen.
private struct HostFooter: View {
    @Environment(AppModel.self) private var model

    var body: some View {
        HStack(spacing: 8) {
            Circle()
                .fill(model.online ? Theme.success : Theme.warning)
                .frame(width: 8, height: 8)
            VStack(alignment: .leading, spacing: 0) {
                Text(model.endpoint?.displayName ?? "")
                    .font(.caption)
                    .lineLimit(1)
                Text(model.online ? (model.via == .linker ? "Linker Connect" : "Connected") : "Reconnecting…")
                    .font(.caption2)
                    .foregroundStyle(Theme.muted)
            }
            Spacer()
            Button("Disconnect") { model.disconnect() }
                .font(.caption)
                .buttonStyle(.borderless)
                .accessibilityIdentifier("disconnect")
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(.bar)
    }
}

extension String {
    var nilIfEmpty: String? { isEmpty ? nil : self }
}
