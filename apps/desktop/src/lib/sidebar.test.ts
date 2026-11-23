import { describe, expect, it } from "vitest";
import type { ProjectSummary, ThreadSummary } from "#shared/protocol.ts";
import { deriveProjectIdentity, firstEmoji, monogramText, parseProjectIcon } from "#shared/project-icon.ts";
import {
  createWorkingTracker,
  isUnread,
  isWoke,
  listedThreads,
  machineProjects,
  nextThreadAfter,
  partitionSidebar,
  relativeTime,
  scopedProjects,
  searchThreads,
  snoozePresets,
  sortProjects,
  topStatus,
  workingDuration,
  WORKING_EXIT_GRACE_MS,
  type SidebarProject,
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

function summary(id: string, threads: ThreadSummary[], fields: Partial<ProjectSummary> = {}): ProjectSummary {
  return { id, name: id, dir: `/projects/${id}`, link: null, local: {}, open: false, threads, icon: null, pinned: false, archived: false, setup: false, autonomous: false, ...fields };
}

/** A project on this computer, as the sidebar lists it. */
function project(id: string, threads: ThreadSummary[], fields: Partial<ProjectSummary> = {}): SidebarProject {
  return machineProjects([{ id: "this", projects: [summary(id, threads, fields)], offline: false }])[0]!;
}

const ids = (threads: readonly { id: string }[]) => threads.map((entry) => entry.id);

describe("sidebar shelves (T3 Code's inbox)", () => {
  it("puts pinned threads on top, then Active, Working, Snoozed and Settled, every project's mixed", () => {
    const lists = partitionSidebar(
      [
        project("a", [
          thread("pinned", { pinnedAt: at(5), busy: true }),
          thread("active", { lastSentAt: at(30) }),
          thread("working", { busy: true }),
          thread("approval", { busy: true, waiting: true, lastSentAt: at(20) }),
          thread("settled", { settledAt: at(3) }),
          thread("snoozed", { snoozedUntil: at(-60) }),
          thread("woke", { snoozedUntil: at(1), lastSentAt: at(40) }),
          thread("archived", { archived: true }),
        ]),
        project("b", [thread("other", { lastSentAt: at(10) })]),
      ],
      NOW,
    );
    expect(ids(lists.pinned)).toEqual(["pinned"]);
    // Asking for an approval needs you, so it's Active, not Working; a woken thread is Active again.
    expect(ids(lists.active)).toEqual(["other", "approval", "active", "woke"]);
    expect(ids(lists.working)).toEqual(["working"]);
    expect(ids(lists.snoozed)).toEqual(["snoozed"]);
    expect(ids(lists.settled)).toEqual(["settled"]);
    expect(lists.folders).toBeNull();
    expect(ids(lists.order)).toEqual(["pinned", "other", "approval", "active", "woke", "working"]);
  });

  it("sorts Active by when each came back, and Working by the user's last message", () => {
    const lists = partitionSidebar(
      [
        project("a", [
          thread("sent", { lastSentAt: at(30) }),
          thread("finished", { lastSentAt: at(60), turnEndedAt: at(5) }),
          thread("unsettled", { unsettledAt: at(1) }),
          thread("slow", { busy: true, lastSentAt: at(50) }),
          thread("quick", { busy: true, lastSentAt: at(2) }),
        ]),
      ],
      NOW,
    );
    expect(ids(lists.active)).toEqual(["unsettled", "finished", "sent"]);
    expect(ids(lists.working)).toEqual(["quick", "slow"]);
  });

  it("wakes a snoozed thread early when it asks for an approval", () => {
    const asking = thread("asking", { snoozedUntil: at(-60), snoozedAt: at(10), busy: true, waiting: true });
    expect(ids(partitionSidebar([project("a", [asking])], NOW).active)).toEqual(["asking"]);
    expect(topStatus(asking, NOW)).toBe("approval");
  });

  it("sorts the settled shelf by when they were settled, newest first", () => {
    const lists = partitionSidebar([project("a", [thread("first", { settledAt: at(60) }), thread("last", { settledAt: at(5) })])], NOW);
    expect(ids(lists.settled)).toEqual(["last", "first"]);
  });

  it("lists only the picked projects' threads, and every project when none of them is listed", () => {
    const projects = [project("a", [thread("one"), thread("done", { settledAt: at(1) })]), project("b", [thread("two")])];
    const scoped = partitionSidebar(projects, NOW, { scope: new Set(["this:a"]) });
    expect([...ids(scoped.active), ...ids(scoped.settled)]).toEqual(["one", "done"]);
    expect(ids(partitionSidebar(projects, NOW, { scope: new Set(["this:gone"]) }).active)).toEqual(["one", "two"]);
    expect(scopedProjects(projects, new Set(["this:b"])).map((entry) => entry.key)).toEqual(["this:b"]);
  });

  it("opens the next listed thread after the open one is settled, wrapping around", () => {
    const order = partitionSidebar([project("a", [thread("one", { lastSentAt: at(1) }), thread("two", { lastSentAt: at(2) })])], NOW).order;
    expect(nextThreadAfter(order, "this:one")?.id).toBe("two");
    expect(nextThreadAfter(order, "this:two")?.id).toBe("one");
  });
});

describe("the Working shelf's grace", () => {
  const listedOne = (fields: Partial<ThreadSummary>) => partitionSidebar([project("a", [thread("t", { lastSentAt: at(30), ...fields })])], NOW).working;
  /** The page looks at the thread at `now`, as it is then. */
  const look = (tracker: ReturnType<typeof createWorkingTracker>, fields: Partial<ThreadSummary>, now: number) => {
    const projects = [project("a", [thread("t", { lastSentAt: at(30), ...fields })])];
    return partitionSidebar(projects, NOW, { working: tracker.observe(listedThreads(projects), now) });
  };

  it("keeps a thread whose turn just ended on the Working shelf for a moment, then puts it on top of Active", () => {
    expect(ids(listedOne({ busy: true }))).toEqual(["t"]);
    const tracker = createWorkingTracker();
    const start = NOW;
    expect(ids(look(tracker, { busy: true }, start).working)).toEqual(["t"]);
    const ended = { busy: false, turnEndedAt: new Date(start + 500).toISOString() };
    expect(ids(look(tracker, ended, start + 500).working)).toEqual(["t"]);
    expect(tracker.nextDeadline()).toBe(start + 500 + WORKING_EXIT_GRACE_MS);
    expect(ids(look(tracker, ended, start + 500 + WORKING_EXIT_GRACE_MS).active)).toEqual(["t"]);
    expect(tracker.nextDeadline()).toBeNull();
  });

  it("moves it at once when it asks for an approval or the user settles it", () => {
    const tracker = createWorkingTracker();
    look(tracker, { busy: true }, NOW);
    expect(ids(look(tracker, { busy: true, waiting: true }, NOW + 100).active)).toEqual(["t"]);
    const other = createWorkingTracker();
    look(other, { busy: true }, NOW);
    expect(ids(look(other, { busy: false, settledAt: at(0) }, NOW + 100).settled)).toEqual(["t"]);
    expect(other.nextDeadline()).toBeNull();
  });

  it("gives no grace to a turn that ended long before the page saw it", () => {
    const tracker = createWorkingTracker();
    look(tracker, { busy: true }, NOW);
    expect(ids(look(tracker, { busy: false, turnEndedAt: at(5) }, NOW).active)).toEqual(["t"]);
  });
});

describe("project folders (T3 Code's boring sidebar)", () => {
  it("puts active threads in their project, running ones too, with no Working shelf", () => {
    const lists = partitionSidebar(
      [project("a", [thread("pinned", { pinnedAt: at(5) }), thread("active"), thread("running", { busy: true, lastSentAt: at(1) }), thread("settled", { settledAt: at(3) })])],
      NOW,
      { folders: true },
    );
    expect(ids(lists.pinned)).toEqual(["pinned"]);
    expect(lists.folders!.map((folder) => ids(folder.threads))).toEqual([["running", "active"]]);
    expect(lists.working).toEqual([]);
    expect(lists.active).toEqual([]);
    expect(ids(lists.order)).toEqual(["pinned", "running", "active"]);
  });

  it("sorts threads by the user's last message, and projects by their newest thread", () => {
    const lists = partitionSidebar(
      [
        project("quiet", [thread("old", { lastSentAt: at(300) })]),
        project("busy", [thread("older", { lastSentAt: at(200) }), thread("newest", { lastSentAt: at(1) }), thread("new", { createdAt: at(10) })]),
      ],
      NOW,
      { folders: true },
    );
    expect(lists.folders!.map((folder) => folder.project.id)).toEqual(["busy", "quiet"]);
    expect(ids(lists.folders![0]!.threads)).toEqual(["newest", "new", "older"]);
  });

  it("lists pinned projects first and archived ones last, and drops projects without active threads unless picked", () => {
    const projects = [
      project("archived", [thread("x", { lastSentAt: at(1) })], { archived: true }),
      project("recent", [thread("y", { lastSentAt: at(2) })]),
      project("pinned", [thread("z", { settledAt: at(1), lastSentAt: at(500) })], { pinned: true }),
      project("settled", [thread("w", { settledAt: at(1) })]),
      project("open in Resolve", [], { open: true }),
    ];
    expect(sortProjects(projects).map((entry) => entry.id)).toEqual(["pinned", "recent", "settled", "open in Resolve", "archived"]);
    const folders = (scope?: Set<string>) => partitionSidebar(projects, NOW, { folders: true, scope }).folders!.map((folder) => folder.project.id);
    expect(folders()).toEqual(["pinned", "recent", "open in Resolve", "archived"]);
    expect(folders(new Set(["this:settled"]))).toEqual(["settled"]);
  });
});

describe("several machines", () => {
  // Two machines can have the same ids: a project folder copied over, or a thread id that happens to match.
  const projects = machineProjects([
    { id: "this", projects: [summary("p", [thread("t", { lastSentAt: at(1), busy: true }), thread("s", { settledAt: at(9) })], { name: "Promo" })], offline: false },
    { id: "rig", projects: [summary("p", [thread("t", { lastSentAt: at(2), busy: true }), thread("u", { lastSentAt: at(4) })], { name: "Promo" })], offline: false },
    { id: "laptop", projects: [summary("q", [thread("u", { lastSentAt: at(3), busy: true, waiting: true })], { name: "Old" })], offline: true },
  ]);

  it("mixes every machine's threads on the shelves, each keyed by its machine", () => {
    const lists = partitionSidebar(projects, NOW);
    expect(lists.working.map((entry) => entry.key)).toEqual(["this:t", "rig:t"]);
    expect(lists.active.map((entry) => entry.key)).toEqual(["laptop:u", "rig:u"]);
    expect(lists.settled.map((entry) => entry.key)).toEqual(["this:s"]);
    expect(nextThreadAfter(lists.order, "rig:u")?.key).toBe("this:t");
    expect(partitionSidebar(projects, NOW, { scope: new Set(["rig:p"]) }).order.map((entry) => entry.key)).toEqual(["rig:u", "rig:t"]);
  });

  it("shows two machines' turns working at once, and an offline machine's as they were, without a turn running", () => {
    const status = partitionSidebar(projects, NOW).order.map((entry) => [entry.key, topStatus(entry, NOW), entry.project.offline]);
    expect(status).toEqual([
      ["laptop:u", null, true],
      ["rig:u", null, false],
      ["this:t", "working", false],
      ["rig:t", "working", false],
    ]);
  });

  it("lists every machine's projects in folders too", () => {
    expect(partitionSidebar(projects, NOW, { folders: true }).folders!.map((folder) => folder.project.key)).toEqual(["this:p", "rig:p", "laptop:q"]);
  });

  it("finds threads on every machine", () => {
    expect(searchThreads(projects, "promo").map((entry) => entry.key)).toEqual(["this:t", "rig:t", "rig:u", "this:s"]);
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
    expect([12_400, 245_000, 3_900_000].map(workingDuration)).toEqual(["12s", "4m", "1h 5m"]);
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
