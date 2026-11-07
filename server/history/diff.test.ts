import { describe, expect, it } from "vitest";
import { describeChanges } from "./diff.ts";
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
