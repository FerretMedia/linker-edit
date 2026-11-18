import Foundation
import XCTest
@testable import LinkerEditKit

/// The messages in Fixtures/ were recorded from the real server (crates/server/src/http/ipad_fixtures.rs).
/// A protocol change that breaks the iPad fails here once they are recorded again.
final class FixtureTests: XCTestCase {
    private func fixtures(_ folder: String) throws -> [URL] {
        let root = try XCTUnwrap(Bundle.module.url(forResource: "Fixtures", withExtension: nil))
        let files = try FileManager.default.contentsOfDirectory(at: root.appendingPathComponent(folder), includingPropertiesForKeys: nil)
        return files.filter { $0.pathExtension == "json" }.sorted { $0.lastPathComponent < $1.lastPathComponent }
    }

    func testDecodesEveryServerMessage() throws {
        let files = try fixtures("server")
        XCTAssertGreaterThan(files.count, 10)
        for file in files {
            let message = try ServerMessage.decode(Data(contentsOf: file))
            if case .ignored(let type) = message {
                XCTFail("\(file.lastPathComponent): \(type) is a message the iPad should read")
            }
        }
    }

    func testReplaysTheRecordedSession() throws {
        var state: AppState?
        var states: [AppState] = []
        var refused: [String] = []
        var toasts: [String] = []
        for file in try fixtures("server") {
            let message = try ServerMessage.decode(Data(contentsOf: file))
            if case .refused(_, let text) = message { refused.append(text) }
            if case .toast(let text, _) = message { toasts.append(text) }
            state = applyServerMessage(state, message)
            if let state { states.append(state) }
        }

        let first = try XCTUnwrap(states.first)
        XCTAssertEqual(first.meta.projects.map(\.name), ["Interview", "Promo"])
        XCTAssertEqual(first.meta.projects.first?.threads.map(\.title), ["Silences"])
        XCTAssertEqual(first.meta.threadId, "thread-1")
        XCTAssertEqual(first.meta.resolve, .checking)
        XCTAssertEqual(first.meta.computer, "edit-pc")

        XCTAssertTrue(states.contains { state in
            if case .connected(_, let version, let project, let timeline, let database) = state.meta.resolve {
                return version == "21.1.2" && project == "Interview" && timeline == "Cut" && database?.name == "Local Database"
            }
            return false
        })

        // The approval the turn waited for.
        let asking = try XCTUnwrap(states.first { !$0.meta.approvals.isEmpty })
        XCTAssertEqual(asking.meta.approvalMode, .ask)
        XCTAssertTrue(asking.meta.busy)
        XCTAssertEqual(asking.meta.approvals.first?.tool, "mcp__linker__create_timeline")
        XCTAssertEqual(asking.meta.approvals.first?.input["name"], .string("AI - Cut"))

        // Streaming: the assistant's text grows in place, and the tool goes from running to done.
        let done = try XCTUnwrap(states.last { $0.meta.threadId == "thread-1" })
        XCTAssertFalse(done.meta.busy)
        XCTAssertEqual(done.items.map(\.id).filter { !$0.hasPrefix("00000000") }, ["a1", "t1", "t2", "a2"])
        guard case .assistant(_, let text)? = done.items.first(where: { $0.id == "a1" }) else { return XCTFail("no a1") }
        XCTAssertEqual(text, "Looking at the timeline first.")
        guard case .tool(let call)? = done.items.first(where: { $0.id == "t1" }) else { return XCTFail("no t1") }
        XCTAssertEqual(call.status, .done)
        XCTAssertEqual(call.output, "[\"Cut\"]")
        XCTAssertTrue(done.items.contains { item in
            if case .notice(_, _, let tone) = item { return tone == .info }
            return false
        })
        guard case .user(_, let sent)? = done.items.first else { return XCTFail("no user message first") }
        XCTAssertEqual(sent, "Cut the silences")

        XCTAssertTrue(states.contains { state in
            if case .rendering(let timeline, let percent) = state.meta.preview { return timeline?.name == "AI - Cut" && percent == 40 }
            return false
        })
        guard case .ready(_, let url, _, let outdated) = done.meta.preview else { return XCTFail("preview not ready") }
        XCTAssertEqual(url, "/preview/preview-1.mp4")
        XCTAssertFalse(outdated)

        // Switching threads: a new snapshot with the other thread's (empty) chat, and the refused message.
        let last = try XCTUnwrap(state)
        XCTAssertEqual(last.meta.threadId, "thread-2")
        XCTAssertEqual(last.meta.project?.name, "Promo")
        XCTAssertTrue(last.items.isEmpty)
        XCTAssertNotNil(last.meta.blocked)
        XCTAssertEqual(refused, ["Add titles"])
        XCTAssertEqual(toasts.count, 1)
    }

    /// What the iPad sends is what the server's own types say (Fixtures/client, typed against ClientMessage).
    func testEncodesClientMessagesAsTheServerExpects() throws {
        let messages: [String: ClientMessage] = [
            "send": .send(text: "Cut the silences"),
            "stop": .stop,
            "approve": .approve(id: "request-1", allow: true),
            "deny": .approve(id: "request-1", allow: false),
            "select_thread": .selectThread(threadId: "thread-1"),
            "new_thread": .newThread(projectId: "project-1"),
            "set_approval_mode": .setApprovalMode(.ask),
            "set_provider": .setProvider(.codex),
            "refresh_resolve": .refreshResolve,
            "render_preview": .renderPreview(timelineId: nil),
            "render_preview_timeline": .renderPreview(timelineId: "timeline-1"),
            "stop_preview": .stopPreview,
            "mark_read": .markRead(threadId: "thread-1", read: true),
        ]
        let files = try fixtures("client")
        XCTAssertEqual(Set(files.map { $0.deletingPathExtension().lastPathComponent }), Set(messages.keys))
        for file in files {
            let name = file.deletingPathExtension().lastPathComponent
            let expected = try JSONSerialization.jsonObject(with: Data(contentsOf: file)) as? NSDictionary
            let encoded = try JSONSerialization.jsonObject(with: JSONEncoder().encode(messages[name]!)) as? NSDictionary
            XCTAssertEqual(encoded, expected, name)
        }
    }
}
