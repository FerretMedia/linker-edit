import { describe, expect, it } from "vitest";
import type { PlayableClip, PlayableTimeline } from "#shared/protocol.ts";
import { clipsAhead, clipsAt, formatTimecode, lookStyle, nearestCut } from "./player.ts";

const clip = (name: string, start: number, end: number, track = 1): PlayableClip => ({
  name,
  track,
  start,
  end,
  offset: 0,
  rate: 1,
  media: name,
  kind: "video",
  look: {},
});

// a [0–2) b [2–5) on V1, a gap until 6, c [6–8); a title on V2 over [1–3).
const video = [clip("a", 0, 2), clip("title", 1, 3, 2), clip("b", 2, 5), clip("c", 6, 8)];
const timeline: PlayableTimeline = { id: "t", name: "T", commit: null, fps: 24, width: 1920, height: 1080, duration: 10, video, audio: [], markers: [] };
const names = (clips: PlayableClip[]) => clips.map((c) => c.name);

describe("player", () => {
  it.each([
    [0, ["a"]],
    [1.5, ["a", "title"]],
    [2, ["b", "title"]],
    [5.5, []],
    [8, []],
  ])("shows the top of each track at %ss", (time, expected) => {
    expect(names(clipsAt(video, time))).toEqual(expected);
  });

  it("loads the clips starting in the next few seconds", () => {
    expect(names(clipsAhead(video, 1.5))).toEqual(["b"]);
    expect(names(clipsAhead(video, 4))).toEqual(["c"]);
  });

  it.each<[number, 1 | -1, number]>([
    [0, 1, 1],
    [1, 1, 2],
    [2.01, 1, 3],
    [5.5, 1, 6],
    [8, 1, 10],
    [10, 1, 10],
    [2, -1, 1],
    [2.01, -1, 1],
    [0.5, -1, 0],
  ])("from %ss, the cut in direction %i is at %ss", (time, direction, expected) => {
    expect(nearestCut(timeline, time, direction)).toBe(expected);
  });

  it.each([
    [0, "0:00:00"],
    [6.5, "0:06:12"],
    [61 + 23 / 24, "1:01:23"],
    [3600 * 2 + 5, "2:00:05:00"],
  ])("writes %ss as %s", (seconds, expected) => {
    expect(formatTimecode(seconds, 24)).toBe(expected);
  });

  it("turns Resolve's transform into CSS", () => {
    expect(lookStyle({ ZoomX: 1.3, ZoomY: 1.3, Pan: 192, Tilt: 108, RotationAngle: 90, Opacity: 50, FlipX: true }, 1920, 1080)).toBe(
      "transform: translate(10%, -10%) rotate(-90deg) scale(-1.3, 1.3); opacity: 0.5;",
    );
  });
});
