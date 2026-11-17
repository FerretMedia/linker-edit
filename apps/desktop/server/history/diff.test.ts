import { describe, expect, it } from "vitest";
import { describeChanges, timelineStrips } from "./diff.ts";
import { timelineFolders, type Clip, type StoredTimeline } from "./model.ts";

const clip = (id: string, overrides: Partial<Clip> = {}): Clip => ({
  track: 1,
  id,
  name: id,
  start: 0,
  end: 100,
  source_start: 0,
  source_end: 100,
  media: `m-${id}`,
  ...overrides,
});

const timeline = (overrides: Partial<StoredTimeline> = {}, name = "Cut", endFrame = 2400): StoredTimeline => ({
  metadata: {
    id: "t1",
    name,
    frame_rate: 24,
    width: 1920,
    height: 1080,
    start_frame: 0,
    end_frame: endFrame,
    start_timecode: "01:00:00:00",
    video_tracks: ["V1"],
    audio_tracks: ["A1"],
  },
  cuts: [clip("a"), clip("b", { start: 100, end: 200 })],
  audio: [],
  markers: [],
  ...overrides,
});

describe("describeChanges", () => {
  it.each<[string, StoredTimeline[], StoredTimeline[], string[]]>([
    ["nothing changed", [timeline()], [timeline()], []],
    ["a new timeline", [], [timeline()], ['"Cut": new timeline, 2 clips, 1:40']],
    ["a removed timeline", [timeline()], [], ['"Cut": timeline removed']],
    ["a rename, matched by id", [timeline()], [timeline({}, "Cut v2")], ['"Cut v2": renamed from "Cut"']],
    [
      "clips added and removed, with the new length",
      [timeline()],
      [timeline({ cuts: [clip("a"), clip("c"), clip("d")] }, "Cut", 1200)],
      ['"Cut": 2 clips added (c, d), 1 clip removed (b), length 1:40 → 0:50'],
    ],
    [
      "trimmed beats moved, moved beats changed",
      [timeline()],
      [timeline({ cuts: [clip("a", { source_start: 10 }), clip("b", { start: 300, end: 400, properties: { ZoomX: 2 } })] })],
      ['"Cut": 1 clip trimmed (a), 1 clip moved (b)'],
    ],
    [
      "property and enabled changes",
      [timeline()],
      [timeline({ cuts: [clip("a", { properties: { Opacity: 50 } }), clip("b", { start: 100, end: 200, enabled: false })] })],
      ['"Cut": 2 clips changed (a, b)'],
    ],
    [
      "a clip placed again under a new id is still that clip, matched by media and source point",
      [timeline()],
      [timeline({ cuts: [clip("a"), clip("b2", { media: "m-b", start: 148, end: 248 }), clip("c", { media: "m-b", source_start: 500, source_end: 600 })] })],
      ['"Cut": 1 clip added (c), 1 clip moved (b2)'],
    ],
    [
      "audio clips are counted apart from video",
      [timeline({ audio: [clip("x")] })],
      [timeline({ audio: [] })],
      ['"Cut": 1 audio clip removed (x)'],
    ],
    [
      "long lists drop the names",
      [timeline({ cuts: [] })],
      [timeline({ cuts: ["p", "q", "r", "s"].map((id) => clip(id)) })],
      ['"Cut": 4 clips added'],
    ],
    [
      "markers by frame",
      [timeline({ markers: [{ frame: 10, color: "Blue", name: "a", note: "", duration: 1 }] })],
      [timeline({ markers: [{ frame: 10, color: "Red", name: "a", note: "", duration: 1 }, { frame: 20, color: "Blue", name: "b", note: "", duration: 1 }] })],
      ['"Cut": 1 marker added, 1 marker changed'],
    ],
  ])("%s", (_name, before, after, expected) => {
    expect(describeChanges(before, after)).toEqual(expected);
  });
});

describe("timelineStrips", () => {
  // Ten 1-second clips back to back at 24 fps; "c3" is a still.
  const row = (count: number) => Array.from({ length: count }, (_, i) => clip(`c${i}`, { start: i * 24, end: i * 24 + 24 }));
  const cut = (cuts: Clip[]) => timeline({ cuts });
  const isImage = (media: string) => media === "m-c3";
  // "c3:moved+2" for a clip, "+4" for folded clips.
  const short = (before: StoredTimeline[], after: StoredTimeline[]) =>
    timelineStrips(before, after, isImage).map((strip) => ({
      state: strip.state,
      tiles: strip.tiles
        .map((tile) => (tile.kind === "more" ? `+${tile.count}` : `${tile.name}${tile.change ? `:${tile.change}` : ""}${tile.delta ?? ""}`))
        .join(" "),
    }));
  const moved = row(10).map((c) => (c.id === "c5" ? { ...c, start: c.start + 48, end: c.end + 48 } : c));

  it.each<[string, StoredTimeline[], StoredTimeline[], { state: string; tiles: string }[]]>([
    ["a new timeline shows its whole cut", [], [cut(row(3))], [{ state: "new", tiles: "c0 c1 c2" }]],
    ["a long new timeline folds past the limit", [], [cut(row(30))], [{ state: "new", tiles: `${row(24).map((c) => c.id).join(" ")} +6` }]],
    ["a move keeps a neighbour each side and folds the rest", [cut(row(10))], [cut(moved)], [{ state: "changed", tiles: "+5 c6 c5:moved2 c7 +2" }]],
    [
      "a trim shows the change in length, a removal shows where the clip was",
      [cut(row(4))],
      [cut([{ ...row(4)[0]!, end: 12 }, ...row(4).slice(1, 3)])],
      [{ state: "changed", tiles: "c0:trimmed-0.5 c1 c2 c3:removed" }],
    ],
    ["a lone untouched clip is not folded", [cut(row(3))], [cut([row(3)[0]!, row(3)[1]!, { ...row(3)[2]!, name: "renamed" }])], [{ state: "changed", tiles: "c0 c1 renamed:changed" }]],
    ["no strip when the clips did not change", [cut(row(3))], [timeline({ cuts: row(3), markers: [{ frame: 1, color: "Blue", name: "", note: "", duration: 1 }] })], []],
  ])("%s", (_name, before, after, expected) => {
    expect(short(before, after)).toEqual(expected);
  });

  it("marks stills so the panel can show them", () => {
    const [strip] = timelineStrips([], [cut(row(4))], isImage);
    expect(strip!.tiles.map((tile) => tile.kind === "clip" && tile.image)).toEqual([false, false, false, true]);
  });
});

describe("timelineFolders", () => {
  it("makes names safe on every OS and keeps them unique", () => {
    expect(timelineFolders(["AI/Rough cut", "AI_Rough cut", "a:b?", "Trailing. ", "///"])).toEqual([
      "AI_Rough cut",
      "AI_Rough cut (2)",
      "a_b_",
      "Trailing",
      "___",
    ]);
  });
});
