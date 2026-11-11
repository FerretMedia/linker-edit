import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { createHistory, resolveWriteAction } from "./history.ts";
import type { RawSnapshot, RawTimeline } from "./model.ts";

const item = (id: string, start: number, properties: Record<string, unknown> = {}) => ({
  id,
  name: `clip ${id}`,
  start,
  end: start + 48,
  source_start: 0,
  source_end: 48,
  media: { id: `m-${id}`, name: `${id}.mov`, path: `D:\\footage\\${id}.mov` },
  enabled: true,
  properties: { ZoomX: 1, Opacity: 100, ...properties },
});

const rawTimeline = (id: string, name: string, items: ReturnType<typeof item>[]): RawTimeline => ({
  id,
  name,
  frame_rate: "24",
  width: "1920",
  height: "1080",
  start_frame: 86400,
  end_frame: 86400 + items.length * 48,
  start_timecode: "01:00:00:00",
  video: [{ index: 1, name: "Video 1", items }],
  audio: [{ index: 1, name: "Audio 1", items: [] }],
  markers: [],
});

function setup() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-history-"));
  let snapshot: RawSnapshot = { project: "Doc: Final?", current_timeline: null, timelines: [] };
  const history = createHistory({ dir, readSnapshot: async () => snapshot });
  const git = (...args: string[]) => execFileSync("git", args, { cwd: dir, encoding: "utf8" });
  return {
    dir,
    history,
    git,
    show: (timelines: RawTimeline[]) => (snapshot = { ...snapshot, timelines }),
    read: (file: string) => fs.readFileSync(path.join(dir, "resolve", "Doc_ Final_", file), "utf8"),
  };
}

describe("edit history", () => {
  it("commits a snapshot only when Resolve changed, attributing agent turns to the agent", async () => {
    const { history, git, show, read, dir } = setup();
    show([rawTimeline("t1", "Interview", [item("a", 86400), item("b", 86448, { ZoomX: 1.5 })])]);

    expect(await history.record()).toMatchObject({ changes: ['"Interview": new timeline, 2 clips, 0:04'] });
    expect(await history.record()).toBeNull();
    expect(read("Interview/cuts.json")).toBe(
      [
        '{ "items": [',
        '  {"track":1,"id":"a","name":"clip a","start":86400,"end":86448,"source_start":0,"source_end":48,"media":"m-a"},',
        '  {"track":1,"id":"b","name":"clip b","start":86448,"end":86496,"source_start":0,"source_end":48,"media":"m-b","properties":{"ZoomX":1.5}}',
        "] }",
        "",
      ].join("\n"),
    );
    expect(JSON.parse(read("media.json")).media).toHaveLength(2);

    show([
      rawTimeline("t1", "Interview", [item("a", 86400), item("b", 86448, { ZoomX: 1.5 })]),
      rawTimeline("t2", "AI - Tight", [item("b", 86400)]),
    ]);
    const saved = await history.record({
      provider: "claude",
      prompt: "Make a tighter cut\nwith only clip b",
      actions: ['timeline.create_empty {"name":"AI - Tight"}'],
      summary: "Built AI - Tight with clip b.",
      thread: "thread-1",
    });
    expect(saved).toMatchObject({ changes: ['"AI - Tight": new timeline, 1 clip, 0:02'] });
    expect(git("log", "-1", "--format=%an%n%B")).toBe(
      [
        "Claude",
        "Make a tighter cut with only clip b",
        "",
        "Prompt: Make a tighter cut",
        "with only clip b",
        "",
        "Resolve actions:",
        '- timeline.create_empty {"name":"AI - Tight"}',
        "",
        "Changes:",
        '- "AI - Tight": new timeline, 1 clip, 0:02',
        "",
        "Agent summary:",
        "Built AI - Tight with clip b.",
        "",
        "Thread: thread-1",
        "",
        "",
      ].join("\n"),
    );

    const tightSave = (await history.log(1))[0]!.commit;

    show([rawTimeline("t1", "Interview", [item("a", 86400), item("b", 86448, { ZoomX: 1.5 })])]);
    expect(await history.record()).toMatchObject({ changes: ['"AI - Tight": timeline removed'] });
    // A removed timeline still plays from the save that had it; the live one comes from Resolve.
    expect(await history.timeline("t2", tightSave)).toMatchObject({
      name: "AI - Tight",
      commit: tightSave,
      duration: 2,
      video: [{ name: "clip b", start: 0, end: 2, media: "m-b" }],
    });
    expect(await history.timeline("t2", "--output=x")).toBeNull();
    expect(await history.timeline("t2", null)).toBeNull();
    expect(await history.timeline(null, null)).toBeNull();
    expect(await history.timeline("t1", null)).toMatchObject({ commit: null, video: [{ name: "clip a" }, { name: "clip b", look: { ZoomX: 1.5 } }] });
    expect(history.mediaFile("m-a")).toBeNull();
    expect(fs.existsSync(path.join(dir, "resolve", "Doc_ Final_", "AI_Tight"))).toBe(false);
    expect(git("status", "--porcelain")).toBe("");

    expect((await history.recentContext()).replace(/ \d{4}-\d\d-\d\d \d\d:\d\d /g, " DATE ")).toMatch(
      new RegExp(
        [
          String.raw`^\w+ DATE .+: Changes made in Resolve`,
          '  - "AI - Tight": timeline removed',
          String.raw`\w+ DATE Claude: Make a tighter cut with only clip b`,
          '  - "AI - Tight": new timeline, 1 clip, 0:02',
          String.raw`\w+ DATE .+: Changes made in Resolve`,
          '  - "Interview": new timeline, 2 clips, 0:04$',
        ].join("\n"),
      ),
    );
    expect(await history.log()).toMatchObject([
      { agent: null, subject: "Changes made in Resolve", prompt: null, thread: null, actions: [], changes: ['"AI - Tight": timeline removed'], timelines: [] },
      {
        author: "Claude",
        agent: "claude",
        subject: "Make a tighter cut with only clip b",
        prompt: "Make a tighter cut\nwith only clip b",
        actions: ['timeline.create_empty {"name":"AI - Tight"}'],
        changes: ['"AI - Tight": new timeline, 1 clip, 0:02'],
        thread: "thread-1",
        timelines: [{ name: "AI - Tight", state: "new", tiles: [{ kind: "clip", name: "clip b", media: "m-b", image: false, change: null }] }],
        date: expect.stringMatching(/^\d{4}-\d\d-\d\dT\d\d:\d\d:\d\d/),
      },
      { agent: null, changes: ['"Interview": new timeline, 2 clips, 0:04'], timelines: [{ name: "Interview", state: "new" }] },
    ]);
  });

  it("refuses to save another Resolve project into this history, and commits the project file", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-history-"));
    const snapshot: RawSnapshot = {
      project: "Promo",
      project_id: "p-2",
      current_timeline: null,
      timelines: [rawTimeline("t1", "Cut", [item("a", 86400)])],
    };
    const history = createHistory({
      dir,
      readSnapshot: async () => snapshot,
      checkProject: (open) => (open.id === "p-1" ? null : `Resolve has "${open.name}" open.`),
    });
    await expect(history.record()).rejects.toThrow('Resolve has "Promo" open.');
    expect(fs.existsSync(path.join(dir, "resolve"))).toBe(false);

    fs.writeFileSync(path.join(dir, "linker-edit.json"), "{}\n");
    await history.commitProjectFile("Link to Resolve project Doc");
    await history.commitProjectFile("Link to Resolve project Doc");
    expect(execFileSync("git", ["log", "--format=%s"], { cwd: dir, encoding: "utf8" })).toBe("Link to Resolve project Doc\n");
  });

  it("saves nothing for a project without timelines", async () => {
    const { history } = setup();
    expect(await history.record()).toBeNull();
    expect(await history.recentContext()).toBe("");
  });
});

describe("resolveWriteAction", () => {
  it.each([
    ["mcp__resolve__timeline", { action: "create_empty", params: { name: "AI - Cut" } }, 'timeline.create_empty {"name":"AI - Cut"}'],
    ["mcp__resolve__timeline", { action: "set_current" }, "timeline.set_current"],
    ["mcp__resolve__timeline", { action: "list" }, null],
    ["Write", { file_path: "notes.md" }, null],
    [undefined, {}, null],
  ])("%s %j → %s", (tool, input, expected) => {
    expect(resolveWriteAction(tool, input)).toBe(expected);
  });
});
