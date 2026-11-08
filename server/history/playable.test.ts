import { describe, expect, it } from "vitest";
import type { Clip, StoredTimeline } from "./model.ts";
import { playableTimeline } from "./playable.ts";

const clip = (id: string, overrides: Partial<Clip> = {}): Clip => ({
  track: 1,
  id,
  name: id,
  start: 86400,
  end: 86448,
  source_start: 0,
  source_end: 48,
  media: `m-${id}`,
  ...overrides,
});

const timeline = (cuts: Clip[], audio: Clip[] = []): StoredTimeline => ({
  metadata: {
    id: "t1",
    name: "AI - Cut",
    frame_rate: 24,
    width: 1920,
    height: 1080,
    start_frame: 86400,
    end_frame: 86400 + 240,
    start_timecode: "01:00:00:00",
    video_tracks: ["V1", "V2"],
    audio_tracks: ["A1"],
  },
  cuts,
  audio,
  markers: [{ frame: 36, color: "Red", name: "Drop", note: "", duration: 1 }],
});

const entry = (id: string, path: string, fps?: number) => [id, { id, name: "", path, ...(fps ? { fps } : {}) }] as const;
const media = new Map([
  entry("m-a", "D:\\footage\\a.MP4", 25),
  entry("m-b", "D:\\stills\\b.jpg"),
  entry("m-c", "D:\\footage\\c.braw"),
  entry("m-v", "D:\\audio\\v.wav"),
]);

describe("playableTimeline", () => {
  it("places clips in seconds from the timeline start, with what the browser can play", () => {
    const played = playableTimeline(
      timeline(
        [
          clip("b", { start: 86448, end: 86496, properties: { ZoomX: 1.3, ZoomY: 1.3, CropLeft: 10 } }),
          // a's source runs at 25 fps, so its frame 125 is 5 s in.
          clip("a", { source_start: 125, properties: { Speed: 200 } }),
          clip("c", { start: 86496, end: 86544 }),
          clip("gone", { start: 86544, end: 86592 }),
          clip("off", { enabled: false }),
          clip("title", { track: 2, media: null }),
        ],
        [clip("v", { end: 86640 })],
      ),
      media,
      "f8e032f",
    );
    expect(played).toMatchObject({ duration: 10, commit: "f8e032f", markers: [{ time: 1.5, color: "Red", name: "Drop" }] });
    expect(played.video.map(({ name, start, end, offset, rate, kind, media: id, look }) => [name, start, end, offset, rate, kind, id, look])).toEqual([
      ["a", 0, 2, 5, 2, "video", "m-a", {}],
      ["title", 0, 2, 0, 1, "missing", null, {}],
      ["b", 2, 4, 0, 1, "image", "m-b", { ZoomX: 1.3, ZoomY: 1.3 }],
      ["c", 4, 6, 0, 1, "unsupported", "m-c", {}],
      ["gone", 6, 8, 0, 1, "missing", null, {}],
    ]);
    expect(played.audio).toMatchObject([{ name: "v", start: 0, end: 10, kind: "audio" }]);
  });
});
