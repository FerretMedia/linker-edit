import { describe, expect, it } from "vitest";
import type { ProjectSummary, ThreadSummary } from "#shared/protocol.ts";
import { deriveProjectIdentity, firstEmoji, monogramText, parseProjectIcon } from "#shared/project-icon.ts";
import {
  isUnread,
  isWoke,
  nextThreadAfter,
  partitionSidebar,
  relativeTime,
  searchThreads,
  snoozePresets,
  sortProjects,
  topStatus,
} from "./sidebar.ts";

const NOW = Date.parse("2026-11-15T12:00:00.000Z");
const at = (minutesAgo: number) => new Date(NOW - minutesAgo * 60_000).toISOString();

function thread(id: string, fields: Partial<ThreadSummary> = {}): ThreadSummary {
  return {
    id,
    title: id,
    createdAt: at(1000),
    updatedAt: at(500),
    archived: false,
    busy: false,
    waiting: false,
    pinnedAt: null,
    settledAt: null,
    unsettledAt: null,
    snoozedUntil: null,
    snoozedAt: null,
    lastReadAt: null,
    lastSentAt: null,
    turnEndedAt: null,
    ...fields,
  };
}

function project(id: string, threads: ThreadSummary[], fields: Partial<ProjectSummary> = {}): ProjectSummary {
  return { id, name: id, dir: `/projects/${id}`, link: null, local: {}, open: false, threads, icon: null, pinned: false, archived: false, ...fields };
}

describe("sidebar sections", () => {
  it("puts pinned threads on top, active ones in their project, and snoozed and settled ones on their shelves", () => {
    const lists = partitionSidebar(
      [
        project("a", [
          thread("pinned", { pinnedAt: at(5) }),
          thread("active"),
          thread("settled", { settledAt: at(3) }),
          thread("snoozed", { snoozedUntil: at(-60) }),
          thread("woke", { snoozedUntil: at(1) }),
          thread("archived", { archived: true }),
        ]),
      ],
      NOW,
    );
    expect(lists.pinned.map((entry) => entry.id)).toEqual(["pinned"]);
    expect(lists.folders.map((folder) => folder.threads.map((entry) => entry.id))).toEqual([["active", "woke"]]);
    expect(lists.snoozed.map((entry) => entry.id)).toEqual(["snoozed"]);
    expect(lists.settled.map((entry) => entry.id)).toEqual(["settled"]);
    expect(lists.order.map((entry) => entry.id)).toEqual(["pinned", "active", "woke"]);
  });

  it("wakes a snoozed thread early when it asks for an approval", () => {
    const asking = thread("asking", { snoozedUntil: at(-60), snoozedAt: at(10), waiting: true });
    expect(partitionSidebar([project("a", [asking])], NOW).folders[0]!.threads.map((entry) => entry.id)).toEqual(["asking"]);
    expect(topStatus(asking, NOW)).toBe("approval");
  });

  it("sorts threads by the user's last message, and projects by their newest thread", () => {
    const lists = partitionSidebar(
      [
        project("quiet", [thread("old", { lastSentAt: at(300) })]),
        project("busy", [thread("older", { lastSentAt: at(200) }), thread("newest", { lastSentAt: at(1) }), thread("new", { createdAt: at(10) })]),
      ],
      NOW,
    );
    expect(lists.folders.map((folder) => folder.project.id)).toEqual(["busy", "quiet"]);
    expect(lists.folders[0]!.threads.map((entry) => entry.id)).toEqual(["newest", "new", "older"]);
  });

  it("lists pinned projects first and archived ones last, and drops projects without active threads", () => {
    const projects = [
      project("archived", [thread("x", { lastSentAt: at(1) })], { archived: true }),
      project("recent", [thread("y", { lastSentAt: at(2) })]),
      project("pinned", [thread("z", { settledAt: at(1), lastSentAt: at(500) })], { pinned: true }),
      project("settled", [thread("w", { settledAt: at(1) })]),
      project("open in Resolve", [], { open: true }),
    ];
    expect(sortProjects(projects).map((entry) => entry.id)).toEqual(["pinned", "recent", "settled", "open in Resolve", "archived"]);
    expect(partitionSidebar(projects, NOW).folders.map((folder) => folder.project.id)).toEqual(["pinned", "recent", "open in Resolve", "archived"]);
  });

  it("sorts the settled shelf by when they were settled, newest first", () => {
    const lists = partitionSidebar([project("a", [thread("first", { settledAt: at(60) }), thread("last", { settledAt: at(5) })])], NOW);
    expect(lists.settled.map((entry) => entry.id)).toEqual(["last", "first"]);
  });

  it("opens the next listed thread after the open one is settled, wrapping around", () => {
    const order = partitionSidebar([project("a", [thread("one", { lastSentAt: at(1) }), thread("two", { lastSentAt: at(2) })])], NOW).order;
    expect(nextThreadAfter(order, "one")?.id).toBe("two");
    expect(nextThreadAfter(order, "two")?.id).toBe("one");
  });
});

describe("unread and woke", () => {
  it("is unread when a turn ended after the user last looked, and never for a thread never looked at", () => {
    expect(isUnread(thread("a", { turnEndedAt: at(1), lastReadAt: at(5) }))).toBe(true);
    expect(isUnread(thread("a", { turnEndedAt: at(5), lastReadAt: at(1) }))).toBe(false);
    expect(isUnread(thread("a", { turnEndedAt: at(1), lastReadAt: null }))).toBe(false);
    expect(isUnread(thread("a", { turnEndedAt: at(1), lastReadAt: at(5), busy: true }))).toBe(false);
    expect(topStatus(thread("a", { turnEndedAt: at(1), lastReadAt: at(5) }), NOW)).toBe("done");
  });

  it("says Woke until the woken thread is looked at", () => {
    expect(isWoke(thread("a", { snoozedUntil: at(1), lastReadAt: at(30) }), NOW)).toBe(true);
    expect(isWoke(thread("a", { snoozedUntil: at(10), lastReadAt: at(1) }), NOW)).toBe(false);
    expect(isWoke(thread("a", { snoozedUntil: at(-10), lastReadAt: at(30) }), NOW)).toBe(false);
  });
});

describe("search", () => {
  const projects = [
    project("Podcast", [
      thread("1", { title: "Ep 12 rough cut", lastSentAt: at(5) }),
      thread("2", { title: "Color the rough cut", lastSentAt: at(1), settledAt: at(1) }),
      thread("3", { title: "Captions", archived: true }),
    ]),
    project("Launch", [thread("4", { title: "Rough mix" })]),
  ];

  it("finds threads by every word, title starts first, settled and archived ones too", () => {
    expect(searchThreads(projects, "rough").map((entry) => entry.id)).toEqual(["4", "2", "1"]);
    expect(searchThreads(projects, "rough cut").map((entry) => entry.id)).toEqual(["2", "1"]);
    expect(searchThreads(projects, "podcast captions").map((entry) => entry.id)).toEqual(["3"]);
    expect(searchThreads(projects, "  ")).toEqual([]);
  });
});

describe("times", () => {
  it("writes ages the way T3 Code's sidebar does", () => {
    expect(relativeTime(at(0.5), NOW)).toBe("now");
    expect(relativeTime(at(5), NOW)).toBe("5m");
    expect(relativeTime(at(180), NOW)).toBe("3h");
    expect(relativeTime(at(60 * 24 * 9), NOW)).toBe("9d");
  });

  it("offers T3 Code's snooze choices, soonest first, without the ones already past", () => {
    const evening = new Date(2026, 10, 16, 19, 0); // a Monday, after 18:00
    expect(snoozePresets(evening).map((preset) => preset.label)).toEqual(["In 1 hour", "In 3 hours", "Tomorrow", "Next week"]);
    const morning = new Date(2026, 10, 15, 8, 0); // a Sunday
    expect(snoozePresets(morning).map((preset) => preset.label)).toEqual(["In 1 hour", "In 3 hours", "This evening", "Tomorrow"]);
  });
});

describe("project icons", () => {
  it("makes a monogram and a color from the name, like T3 Code", () => {
    expect(deriveProjectIdentity("Wedding Jan").monogram).toBe("WJ");
    expect(deriveProjectIdentity("Episode 12").monogram).toBe("E1");
    expect(deriveProjectIdentity("").monogram).toBe("PR");
    expect(deriveProjectIdentity("Wedding Jan").color).toBe(deriveProjectIdentity(" Wedding Jan ").color);
  });

  it("only accepts well-formed icons", () => {
    expect(parseProjectIcon({ kind: "lucide", name: "rocket", color: "violet" })).toEqual({ kind: "lucide", name: "rocket", color: "violet" });
    expect(parseProjectIcon({ kind: "lucide", name: "Rocket!", color: "violet" })).toBeNull();
    expect(parseProjectIcon({ kind: "monogram", text: "ab", color: "red" })).toEqual({ kind: "monogram", text: "AB", color: "red" });
    expect(parseProjectIcon({ kind: "monogram", text: "abc", color: "red" })).toBeNull();
    expect(parseProjectIcon({ kind: "emoji", emoji: "🎬 film" })).toEqual({ kind: "emoji", emoji: "🎬" });
    expect(parseProjectIcon({ kind: "emoji", emoji: "film" })).toBeNull();
    expect(parseProjectIcon("rocket")).toBeNull();
    expect(monogramText(" x ")).toBe("X");
    expect(firstEmoji("🇳🇱")).toBe("🇳🇱");
  });
});
