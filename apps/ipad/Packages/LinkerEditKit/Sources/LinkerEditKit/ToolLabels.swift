import Foundation

/// How tool calls are named in the chat, as the web UI's labels.ts.
public enum ToolLabels {
    private static let resolvePrefix = "mcp__resolve__"
    private static let linkerPrefix = "mcp__linker__"

    /// Resolve's own MCP server, and Linker-Edit's, whose tools other than transcribe all work in Resolve.
    public static func isResolveTool(_ name: String) -> Bool {
        name.hasPrefix(resolvePrefix) || (name.hasPrefix(linkerPrefix) && name != "\(linkerPrefix)transcribe")
    }

    /// "mcp__linker__create_timeline" → "create_timeline"; other tools keep their name.
    public static func label(_ name: String) -> String {
        guard isResolveTool(name) else { return name }
        let prefix = name.hasPrefix(resolvePrefix) ? resolvePrefix : linkerPrefix
        return String(name.dropFirst(prefix.count))
    }

    private static func script(_ name: String, _ input: JSONValue) -> String? {
        name == "\(resolvePrefix)run_script" ? input["script"]?.string : nil
    }

    /// The one-line detail shown after a tool's label.
    public static func summary(_ name: String, _ input: JSONValue) -> String {
        guard case .object(let record) = input else { return "" }
        if let source = script(name, input) {
            let first = source.split(separator: "\n").first { candidate in
                let trimmed = candidate.trimmingCharacters(in: .whitespaces)
                return !trimmed.isEmpty && !trimmed.hasPrefix("#")
            }
            return first.map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
        }
        if isResolveTool(name) {
            return record.isEmpty ? "" : input.compact
        }
        for key in ["command", "file_path", "path", "pattern", "query", "url", "description"] {
            if let value = record[key]?.string { return value }
        }
        return ""
    }

    /// What an approval shows: a script as code, anything else as JSON.
    public static func approvalDetail(_ name: String, _ input: JSONValue) -> String {
        script(name, input) ?? input.formatted
    }
}
