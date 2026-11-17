import Foundation
import LinkerEditKit

/// A fixed chat for screenshots and UI tests (launch argument -demo), in the server's own JSON.
enum DemoData {
    static func snapshot() -> ServerMessage {
        // Only fails if the JSON below is broken, which the UI tests would show at once.
        try! ServerMessage.decode(Data(json.utf8))
    }

    private static let json = #"""
    {
      "type": "snapshot",
      "state": {
        "projects": [
          {
            "id": "interview", "name": "Interview", "dir": "/projects/interview", "open": true,
            "link": { "kind": "local", "name": "Interview", "projectId": "r-1" },
            "pinned": false, "archived": false,
            "threads": [
              { "id": "silences", "title": "Cut the silences", "updatedAt": "2026-11-17T09:00:00.000Z",
                "archived": false, "busy": true, "waiting": true },
              { "id": "broll", "title": "B-roll over the intro", "updatedAt": "2026-11-16T15:00:00.000Z",
                "archived": false, "busy": false, "waiting": false,
                "turnEndedAt": "2026-11-16T15:00:00.000Z", "lastReadAt": null }
            ]
          },
          {
            "id": "promo", "name": "Spring promo", "dir": "/projects/promo", "open": false,
            "link": { "kind": "network", "name": "Spring promo", "projectId": null },
            "pinned": false, "archived": false,
            "threads": [
              { "id": "titles", "title": "Titles in the brand font", "updatedAt": "2026-11-12T11:00:00.000Z",
                "archived": false, "busy": false, "waiting": false }
            ]
          }
        ],
        "threadId": "silences",
        "projectId": "interview",
        "provider": "claude",
        "defaultProvider": "claude",
        "approvalMode": "ask",
        "busy": true,
        "approvals": [
          { "id": "approval-1", "tool": "mcp__linker__create_timeline",
            "input": { "name": "AI - Interview tight", "from": "Interview", "cuts": 12 } }
        ],
        "resolve": {
          "state": "connected", "product": "DaVinci Resolve Studio", "version": "21.1.2",
          "project": "Interview", "projectId": "r-1", "timeline": "Interview",
          "database": { "type": "Disk", "name": "Local Database" }
        },
        "resolveHere": true,
        "version": "2026.11.17",
        "preview": { "state": "idle" },
        "context": { "usedTokens": 48210, "maxTokens": 200000, "cacheExpiresAt": null },
        "blocked": null,
        "opening": null,
        "computer": "Studio",
        "items": [
          { "id": "u1", "kind": "user", "text": "Cut the silences longer than a second out of the interview." },
          { "id": "a1", "kind": "assistant", "text": "I'll look at the **Interview** timeline first, then find the pauses in its audio." },
          { "id": "t1", "kind": "tool", "name": "mcp__linker__list_timelines", "input": {}, "status": "done",
            "output": "[{\"name\": \"Interview\", \"clips\": 41, \"duration\": \"00:12:31:04\"}]" },
          { "id": "t2", "kind": "tool", "name": "mcp__linker__find_silences", "input": { "timeline": "Interview", "min_seconds": 1 },
            "status": "done", "output": "12 pauses, 47.2 s in all" },
          { "id": "n1", "kind": "notice", "text": "Backed up the project before the first change.", "tone": "info" },
          { "id": "a2", "kind": "assistant", "text": "Found 12 pauses, 47 seconds in all. I'll build **AI - Interview tight** next to your timeline without them." }
        ]
      }
    }
    """#
}
