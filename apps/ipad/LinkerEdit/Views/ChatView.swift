import LinkerEditKit
import SwiftUI

/// The open thread: its messages and tool calls as they stream in, approvals waiting, and the composer floating
/// over them. The chat is content, so it stays on plain backgrounds; only the controls above it are glass.
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
            .safeAreaBar(edge: .bottom) { ComposerView() }
            .onChange(of: growth) {
                withAnimation(.easeOut(duration: 0.15)) { proxy.scrollTo("bottom", anchor: .bottom) }
            }
            .onAppear { proxy.scrollTo("bottom", anchor: .bottom) }
        }
        .navigationTitle(meta?.thread?.title ?? "")
        .navigationBarTitleDisplayMode(.inline)
    }
}

private struct EmptyChat: View {
    var body: some View {
        VStack(spacing: 8) {
            Image(systemName: "bubble.left.and.text.bubble.right")
                .font(.largeTitle)
                .foregroundStyle(.secondary)
            Text("Ask for an edit, such as “cut the silences” or “build a rough cut from the interview”.")
                .multilineTextAlignment(.center)
                .foregroundStyle(.secondary)
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
                    .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18))
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
                .foregroundStyle(tone == .error ? Color.red : Color.secondary)
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
                        .foregroundStyle(call.status == .error ? Color.red : Color.secondary)
                    Text(ToolLabels.label(call.name))
                        .foregroundStyle(call.status == .error ? Color.red : Color.primary)
                    Text(ToolLabels.summary(call.name, call.input))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                    Spacer(minLength: 4)
                    if call.status == .running {
                        ProgressView().controlSize(.mini)
                    } else if call.status == .error {
                        Image(systemName: "xmark").font(.caption).foregroundStyle(.red)
                    }
                    Image(systemName: "chevron.right")
                        .font(.caption2.weight(.semibold))
                        .foregroundStyle(.secondary)
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
                            .overlay(alignment: .leading) { Rectangle().fill(Color(.separator)).frame(width: 2) }
                    }
                }
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
                .padding(.leading, 26)
                .frame(maxWidth: .infinity, alignment: .leading)
            }
        }
        .padding(.horizontal, 12)
        .padding(.vertical, 9)
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 12))
    }
}

/// Supervised mode: the agent waits until the user allows or denies this call. The card is content on a plain
/// background with an orange border so it stands out; only Allow and Deny are glass.
struct ApprovalCard: View {
    let request: ApprovalRequest
    let answer: (Bool) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 8) {
                Image(systemName: "exclamationmark.shield.fill").foregroundStyle(.orange)
                Text("Allow \(ToolLabels.isResolveTool(request.tool) ? "Resolve" : "tool"):").fontWeight(.semibold)
                Text(ToolLabels.label(request.tool)).font(.system(.callout, design: .monospaced))
            }
            ScrollView {
                Text(ToolLabels.approvalDetail(request.tool, request.input))
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
                    .frame(maxWidth: .infinity, alignment: .leading)
            }
            .frame(maxHeight: 240)
            .fixedSize(horizontal: false, vertical: true)
            GlassEffectContainer(spacing: 10) {
                HStack(spacing: 10) {
                    Spacer()
                    Button("Deny") { answer(false) }
                        .buttonStyle(.glass)
                        .accessibilityIdentifier("deny")
                    Button("Allow") { answer(true) }
                        .buttonStyle(.glassProminent)
                        .accessibilityIdentifier("allow")
                }
            }
        }
        .padding(14)
        .background(Color.orange.opacity(0.08), in: RoundedRectangle(cornerRadius: 18))
        .background(Color(.secondarySystemBackground), in: RoundedRectangle(cornerRadius: 18))
        .overlay(RoundedRectangle(cornerRadius: 18).strokeBorder(Color.orange.opacity(0.7), lineWidth: 1.5))
    }
}

private struct WorkingRow: View {
    var body: some View {
        HStack(spacing: 8) {
            ProgressView().controlSize(.small)
            Text("Working…").foregroundStyle(.secondary)
        }
        .font(.callout)
    }
}
