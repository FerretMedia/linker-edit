import Foundation
import XCTest
@testable import LinkerEditKit

final class StoreTests: XCTestCase {
    private func decode(_ json: String) throws -> ServerMessage {
        try ServerMessage.decode(Data(json.utf8))
    }

    private let snapshot = """
    {"type":"snapshot","state":{"projects":[],"threadId":"t1","projectId":null,"provider":"claude","busy":false,
     "approvals":[],"resolve":{"state":"not_running"},"preview":{"state":"idle"},"items":[]}}
    """

    func testAppliesItemsOfTheOpenThreadOnly() throws {
        var state = applyServerMessage(nil, try decode(snapshot))
        state = applyServerMessage(state, try decode(#"{"type":"item","threadId":"t1","item":{"id":"a","kind":"assistant","text":"Hi"}}"#))
        state = applyServerMessage(state, try decode(#"{"type":"item","threadId":"t1","item":{"id":"a","kind":"assistant","text":"Hi there"}}"#))
        state = applyServerMessage(state, try decode(#"{"type":"item","threadId":"other","item":{"id":"b","kind":"user","text":"Elsewhere"}}"#))
        XCTAssertEqual(state?.items, [.assistant(id: "a", text: "Hi there")])
    }

    func testIgnoresMetaAndItemsBeforeTheSnapshot() throws {
        XCTAssertNil(applyServerMessage(nil, try decode(#"{"type":"item","threadId":"t1","item":{"id":"a","kind":"user","text":"x"}}"#)))
        XCTAssertNil(applyServerMessage(nil, try decode(#"{"type":"meta","meta":{"threadId":"t1"}}"#)))
    }

    func testMetaReplacesEverythingButTheChat() throws {
        var state = applyServerMessage(nil, try decode(snapshot))
        state = applyServerMessage(state, try decode(#"{"type":"item","threadId":"t1","item":{"id":"a","kind":"user","text":"x"}}"#))
        state = applyServerMessage(state, try decode(#"{"type":"meta","meta":{"threadId":"t1","busy":true,"approvalMode":"ask","resolve":{"state":"checking"}}}"#))
        XCTAssertEqual(state?.meta.busy, true)
        XCTAssertEqual(state?.meta.approvalMode, .ask)
        XCTAssertEqual(state?.items.count, 1)
    }

    func testUnknownMessagesKindsAndFieldsDontBreakDecoding() throws {
        XCTAssertEqual(try decode(#"{"type":"history","projectId":"p","entries":[]}"#).ignoredType, "history")
        XCTAssertEqual(try decode(#"{"type":"something_new","whatever":[1,2]}"#).ignoredType, "something_new")

        let item = try decode(#"{"type":"item","threadId":"t1","item":{"id":"x","kind":"image","url":"/a.png","extra":{"deep":true}}}"#)
        guard case .item(_, .unknown(let id, let kind)) = item else { return XCTFail("\(item)") }
        XCTAssertEqual(id, "x")
        XCTAssertEqual(kind, "image")

        let meta = try decode("""
        {"type":"meta","meta":{"threadId":"t1","provider":"gemini","approvalMode":"later","newField":{"a":1},
         "resolve":{"state":"sleeping"},"preview":{"state":"uploading"},"busy":"yes",
         "projects":[{"id":"p1","name":"Ok","threads":[{"id":"t1","title":"Fine"},{"title":"no id"}]},{"name":"no id"}],
         "approvals":[{"id":"r1","tool":"Write","input":{"n":[1,null,"x",{"y":false}]}},{"tool":"no id"}]}}
        """)
        guard case .meta(let decoded) = meta else { return XCTFail("\(meta)") }
        XCTAssertEqual(decoded.provider.rawValue, "gemini")
        XCTAssertEqual(decoded.approvalMode.rawValue, "later")
        XCTAssertEqual(decoded.resolve, .unknown(state: "sleeping"))
        XCTAssertEqual(decoded.preview, .unknown(state: "uploading"))
        XCTAssertFalse(decoded.busy)
        XCTAssertEqual(decoded.projects.map(\.id), ["p1"])
        XCTAssertEqual(decoded.projects.first?.threads.map(\.id), ["t1"])
        XCTAssertEqual(decoded.approvals.map(\.id), ["r1"])
        XCTAssertEqual(decoded.approvals.first?.input["n"], .array([.number(1), .null, .string("x"), .object(["y": .bool(false)])]))
    }

    func testAMessageWithoutItsPayloadFailsAlone() {
        XCTAssertThrowsError(try decode(#"{"type":"snapshot"}"#))
        XCTAssertThrowsError(try decode(#"{"type":"item","threadId":"t1"}"#))
        XCTAssertThrowsError(try decode(#"{"no":"type"}"#))
    }

    @MainActor
    func testStoreKeepsToastsAndRefusedMessagesApart() async throws {
        let store = AppStore()
        store.apply(try decode(snapshot))
        store.apply(try decode(#"{"type":"toast","text":"Busy","tone":"error"}"#))
        store.apply(try decode(#"{"type":"toast","text":"Busy","tone":"error"}"#))
        store.apply(try decode(#"{"type":"refused","threadId":"t1","text":"Cut it"}"#))
        XCTAssertEqual(store.toasts.map(\.text), ["Busy"])
        XCTAssertEqual(store.refused, Refused(threadId: "t1", text: "Cut it"))
        XCTAssertEqual(store.state?.meta.threadId, "t1")
        store.reset()
        XCTAssertNil(store.state)
    }

    func testUnreadThreads() throws {
        let read = try JSONDecoder().decode(ThreadSummary.self, from: Data(#"{"id":"t","turnEndedAt":"2026-11-17T09:00:00.000Z","lastReadAt":"2026-11-17T10:00:00.000Z"}"#.utf8))
        let unread = try JSONDecoder().decode(ThreadSummary.self, from: Data(#"{"id":"t","turnEndedAt":"2026-11-17T11:00:00.000Z","lastReadAt":"2026-11-17T10:00:00.000Z"}"#.utf8))
        XCTAssertFalse(read.unread)
        XCTAssertTrue(unread.unread)
    }

    func testToolLabelsMatchTheWebUI() {
        XCTAssertEqual(ToolLabels.label("mcp__linker__create_timeline"), "create_timeline")
        XCTAssertEqual(ToolLabels.label("mcp__resolve__run_script"), "run_script")
        XCTAssertEqual(ToolLabels.label("mcp__linker__transcribe"), "mcp__linker__transcribe")
        XCTAssertEqual(ToolLabels.label("Bash"), "Bash")
        XCTAssertEqual(ToolLabels.summary("Bash", .object(["command": .string("ls -la")])), "ls -la")
        XCTAssertEqual(ToolLabels.summary("mcp__linker__create_timeline", .object(["name": .string("AI - Cut")])), #"{"name":"AI - Cut"}"#)
        XCTAssertEqual(ToolLabels.summary("mcp__linker__list_timelines", .object([:])), "")
        let script = JSONValue.object(["script": .string("# comment\n\nproject = resolve.GetProject()\nmore()")])
        XCTAssertEqual(ToolLabels.summary("mcp__resolve__run_script", script), "project = resolve.GetProject()")
        XCTAssertEqual(ToolLabels.approvalDetail("mcp__resolve__run_script", script), "# comment\n\nproject = resolve.GetProject()\nmore()")
        XCTAssertEqual(ToolLabels.approvalDetail("Write", .string("plain")), "plain")
    }
}

private extension ServerMessage {
    var ignoredType: String? {
        if case .ignored(let type) = self { return type }
        return nil
    }
}
