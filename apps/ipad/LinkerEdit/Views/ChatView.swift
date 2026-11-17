import LinkerEditKit
import SwiftUI

/// The open thread: its messages and tool calls as they stream in, approvals waiting, and the composer.
struct ChatView: View {
    @Environment(AppModel.self) private var model

    private var items: [ChatItem] { model.store.state?.items ?? [] }
    private var meta: AppMeta? { model.store.state?.meta }

    /// Changes whenever the chat grows, also while a message streams in.
    private var growth: Int {
        var hasher = Hasher()
        hasher.combine(items.count)
        hasher.combine(items.last)
        hasher.combine(meta?.approvals.count)
        return hasher.finalize()
    }

    var body: some View {
        VStack(spacing: 0) {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 14) {
                        if items.isEmpty, meta?.busy != true {
                            EmptyChat()
                        }
                        ForEach(items) { item in
                            ChatItemView(item: item)
                        }
                        ForEach(meta?.approvals ?? []) { request in
                            ApprovalCard(request: request) { allow in
                                model.send(.approve(id: request.id, allow: allow))
                            }
                        }
                        if meta?.busy == true && meta?.approvals.isEmpty == true {
                            WorkingRow()
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .padding(.horizontal, 24)
                    .padding(.vertical, 20)
                    .frame(maxWidth: 820)
                    .frame(maxWidth: .infinity)
                }
                .onChange(of: growth) {
                    withAnimation(.easeOut(duration: 0.15)) { proxy.scrollTo("bottom", anchor: .bottom) }
                }
                .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
            }
            ComposerView()
        }
        .background(Theme.background)
        .navigationTitle(meta?.thread?.title ?? "")
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct EmptyChat: View {
    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "bubble.left.and.text.bubble.right")
                .font(.largeTitle)
                .foregroundStyle(Theme.muted)
            Text("Ask for an edit, such as “cut the silences” or “build a rough cut from the interview”.")
                .multilineTextAlignment(.center)
                .foregroundStyle(Theme.muted)
        }
        .frame(maxWidth: .infinity)
        .padding(.top, 80)
    }
}

struct ChatItemView: View {
    let item: ChatItem

    var body: some View {
        switch item {
        case .user(_, let text):
            HStack {
                Spacer(minLength: 80)
                Text(text)
                    .textSelection(.enabled)
                    .padding(.horizontal, 14)
                    .padding(.vertical, 10)
                    .background(Theme.raised, in: RoundedRectangle(cornerRadius: 16))
            }
        case .assistant(_, let text):
            Text(Markdown.inline(text))
                .textSelection(.enabled)
                .frame(maxWidth: .infinity, alignment: .leading)
        case .tool(let call):
            ToolCard(call: call)
        case .notice(_, let text, let tone):
            Label(text, systemImage: tone == .error ? "exclamationmark.triangle" : "info.circle")
                .font(.footnote)
                .foregroundStyle(tone == .error ? Theme.danger : Theme.muted)
        case .unknown:
            EmptyView()
        }
    }
}

enum Markdown {
    /// Bold, italics, code and links; block markdown shows as written.
    static func inline(_ text: String) -> AttributedString {
        let options = AttributedString.MarkdownParsingOptions(interpretedSyntax: .inlineOnlyPreservingWhitespace)
        return (try? AttributedString(markdown: text, options: options)) ?? AttributedString(text)
    }
}

/// A tool call: one line with its name and gist, the input and output when opened.
struct ToolCard: View {
    let call: ToolCall
    @State private var expanded = false

    private var icon: String {
        let name = call.name
        if ToolLabels.isResolveTool(name) { return "film" }
        switch name {
        case "shell", "Bash": return "terminal"
        case "Read": return "doc.text"
        case "Edit", "Write", "edit files": return "square.and.pencil"
        case "Grep", "Glob": return "magnifyingglass"
        case "WebSearch", "WebFetch", "web search": return "globe"
        case "TodoWrite": return "checklist"
        case "ToolSearch": return "book"
        default: return "wrench.and.screwdriver"
        }
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            Button {
                withAnimation(.easeOut(duration: 0.15)) { expanded.toggle() }
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: icon)
                        .frame(width: 18)
                        .foregroundStyle(call.status == .error ? Theme.danger : Theme.muted)
                    Text(ToolLabels.label(call.name))
                        .foregroundStyle(call.status == .error ? Theme.danger : .primary)
                    Text(ToolLabels.summary(call.name, call.input))
                        .foregroundStyle(Theme.muted)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Spacer(minLength: 4)
                    if call.status == .running {
                        ProgressView().controlSize(.mini)
                    } else if call.status == .error {
                        Image(systemName: "xmark").font(.caption).foregroundStyle(Theme.danger)
                    }
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(Theme.muted)
                        .rotationEffect(.degrees(expanded ? 90 : 0))
                }
                .font(.callout)
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("tool-\(call.id)")
            if expanded {
                VStack(alignment: .leading, spacing: 8) {
                    Text(call.input.formatted)
                    if let output = call.output {
                        Text(output)
                            .padding(.leading, 10)
                            .overlay(alignment: .leading) { Rectangle().fill(Theme.border).frame(width: 2) }
                    }
                }
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(Theme.muted)
                .textSelection(.enabled)
                .padding(.leading, 26)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 10))
        .overlay(RoundedRectangle(cornerRadius: 10).strokeBorder(Theme.border))
    }
}

/// Supervised mode: the agent waits until the user allows or denies this call.
struct ApprovalCard: View {
    let request: ApprovalRequest
    let answer: (Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "exclamationmark.shield.fill").foregroundStyle(Theme.warning)
                Text("Allow \(ToolLabels.isResolveTool(request.tool) ? "Resolve" : "tool"):").fontWeight(.semibold)
                Text(ToolLabels.label(request.tool)).font(.system(.callout, design: .monospaced))
            }
            ScrollView {
                Text(ToolLabels.approvalDetail(request.tool, request.input))
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(Theme.muted)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 240)
            .fixedSize(horizontal: false, vertical: true)
            HStack {
                Spacer()
                Button("Deny") { answer(false) }
                    .buttonStyle(.bordered)
                    .accessibilityIdentifier("deny")
                Button("Allow") { answer(true) }
                    .buttonStyle(.borderedProminent)
                    .accessibilityIdentifier("allow")
            }
        }
        .padding(14)
        .background(Theme.card, in: RoundedRectangle(cornerRadius: 16))
        .overlay(RoundedRectangle(cornerRadius: 16).strokeBorder(Theme.warning.opacity(0.35)))
    }
}

private struct WorkingRow: View {
    var body: some View {
        HStack(spacing: 8) {
            ProgressView().controlSize(.small)
            Text("Working…").foregroundStyle(Theme.muted)
        }
        .font(.callout)
    }
}
