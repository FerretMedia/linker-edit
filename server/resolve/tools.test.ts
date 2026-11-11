// Linker-Edit's Resolve tools end to end: the Node side, Resolve's side as real Python against a stand-in
// Resolve (fixtures/DaVinciResolveScript.py), and ffmpeg on generated media.
import { execFileSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { findFfmpeg } from "../transcribe/tools.ts";
import { ResolveError } from "./connection.ts";
import { parseSilences } from "./media.ts";
import { runResolveTool, type ToolContext } from "./run.ts";
import { hasPython, pythonConnection } from "./testing.ts";

const FAKE = fs.readFileSync(path.join(import.meta.dirname, "fixtures", "DaVinciResolveScript.py"), "utf8");

const STATE = {
  project: {
    name: "Doc",
    id: "doc-id",
    current: "Edit",
    timelines: [
      { id: "t1", name: "Edit", video: [[{ id: "i1", start: 90000, end: 90050, media: "m1" }]], audio: [] },
      {
        id: "t2",
        name: "AI - Cut",
        video: [[{ id: "i2", start: 90000, end: 90025, media: "m1" }, { id: "i3", start: 90025, end: 90050, media: "m1" }]],
        audio: [[]],
        markers: [
          { frame: 10, color: "Blue", name: "", note: "", duration: 1 },
          { frame: 20, color: "Red", name: "", note: "", duration: 1 },
          { frame: 30, color: "Red", name: "", note: "", duration: 1 },
        ],
      },
    ],
  },
  media: { name: "Master", clips: [{ id: "m1", name: "m1.mov", path: "D:/m1.mov" }], folders: [{ name: "Interviews", clips: [] }] },
};

function setup() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-tools-"));
  const stateFile = path.join(dir, "state.json");
  fs.writeFileSync(stateFile, JSON.stringify(STATE));
  const connection = pythonConnection(FAKE, { FAKE_RESOLVE: stateFile });
  const project = path.join(dir, "project");
  fs.mkdirSync(project);
  const context: ToolContext = { dir: project, framesRoot: path.join(dir, "frames"), pollMs: 1 };
  const calls = () =>
    fs.existsSync(`${stateFile}.log`)
      ? fs.readFileSync(`${stateFile}.log`, "utf8").trim().split("\n").map((line) => JSON.parse(line) as unknown[])
      : [];
  const tool = async <T = Record<string, unknown>>(name: string, input: Record<string, unknown> = {}) => {
    const result = await runResolveTool(connection, name, input, context);
    return { ...result, json: JSON.parse(result.text) as T };
  };
  return { dir, project, calls, tool };
}

const refusal = (promise: Promise<unknown>) =>
  promise.then(
    () => null,
    (error: unknown) => (error instanceof ResolveError ? `${error.kind}: ${error.message}` : error instanceof Error ? error.message : String(error)),
  );

let ffmpeg: string | null = null;
try {
  ffmpeg = findFfmpeg();
} catch {
  // Without ffmpeg the media tests are skipped.
}

describe("parseSilences", () => {
  const log = (body: string) => `  Duration: 00:00:10.00, start: 0.000000, bitrate: 1 kb/s\n${body}`;

  it("pads silences next to speech, and lists the speech between them", () => {
    const parsed = parseSilences(log("[silencedetect] silence_start: 2\n[silencedetect] silence_end: 4 | silence_duration: 2\n"), 0.25);
    expect(parsed).toEqual({ duration: 10, silences: [[2.25, 3.75]], speech: [[0, 2.25], [3.75, 10]], silent_seconds: 1.5 });
  });

  it("keeps silences at the very start and end of a file whole, and closes one still open", () => {
    const parsed = parseSilences(log("silence_start: -0.01\nsilence_end: 1 | x\nsilence_start: 8.5\n"), 0.1);
    expect(parsed.silences).toEqual([[0, 0.9], [8.6, 10]]);
    expect(parsed.speech).toEqual([[0.9, 8.6]]);
  });
});

describe.skipIf(!hasPython)("Resolve tools", () => {
  it("exports a timeline into the working folder, named after it", async () => {
    const { tool, project, calls } = setup();
    const otio = await tool<{ path: string; relative: string }>("export_timeline", { timeline: "AI - Cut" });
    expect(otio.json.relative).toBe("exports/AI - Cut.otio");
    expect(fs.readFileSync(path.join(project, "exports", "AI - Cut.otio"), "utf8")).toBe("exported");
    const edl = await tool<{ relative: string }>("export_timeline", { format: "edl", name: "cut: list" });
    expect(edl.json.relative).toBe("exports/cut- list.edl");
    expect(calls()).toEqual([
      ["Export", "AI - Cut", "AI - Cut.otio", "otio", "none"],
      ["Export", "Edit", "cut- list.edl", "edl", "none"],
    ]);
    expect(await refusal(tool("export_timeline", { format: "mp4" }))).toMatch(/^Export formats are otio/);
  });

  it("imports a timeline file as a new AI timeline linked to the media pool", async () => {
    const { tool, project, calls } = setup();
    fs.writeFileSync(path.join(project, "rebuilt.otio"), "{}");
    const result = await tool<{ name: string; items: object }>("import_timeline", { path: "rebuilt.otio", name: "Rebuilt" });
    expect(result.json).toMatchObject({ name: "AI - Rebuilt", items: { video: 1, audio: 0 } });
    expect(calls()).toEqual([
      ["ImportTimelineFromFile", "rebuilt.otio", { timelineName: "AI - Rebuilt", importSourceClips: false, sourceClipsFolders: ["Master", "Interviews"] }],
      ["SetName", "Timeline 1", "AI - Rebuilt"],
      ["SetCurrentTimeline", "AI - Rebuilt"],
    ]);
    fs.writeFileSync(path.join(project, "clip.mp4"), "");
    expect(await refusal(tool("import_timeline", { path: "clip.mp4", name: "X" }))).toMatch(/reads \.otio/);
    expect(await refusal(tool("import_timeline", { path: "missing.otio", name: "X" }))).toMatch(/There is no file missing\.otio/);
  });

  it.skipIf(!ffmpeg)("shows timeline frames as small JPEGs, putting the playhead back", async () => {
    const { tool, calls } = setup();
    const result = await tool<{ frames: { frame: number; ok: boolean; file: string }[] }>("view_frames", { timeline: "AI - Cut", frames: [90000, 90030] });
    expect(result.json.frames.map(({ frame, ok }) => ({ frame, ok }))).toEqual([
      { frame: 90000, ok: true },
      { frame: 90030, ok: true },
    ]);
    expect(result.images).toHaveLength(2);
    for (const image of result.images!) {
      expect(image.endsWith(".jpg")).toBe(true);
      expect(fs.existsSync(image)).toBe(true);
      expect(fs.existsSync(image.replace(/\.jpg$/, ".png"))).toBe(false);
    }
    const playhead = calls().filter((call) => call[0] === "SetCurrentTimecode").map((call) => call[2]);
    // 25 fps: 90000 is one hour in; the last move puts the playhead back where it was.
    expect(playhead).toEqual(["01:00:00:00", "01:00:01:05", "01:00:00:00"]);
  });

  it.skipIf(!ffmpeg)("shows moments of a media file, and finds its silences", async () => {
    const { tool, dir } = setup();
    const file = path.join(dir, "talk.mkv");
    // One second of tone, two of silence, one of tone, with a picture.
    execFileSync(ffmpeg!, [
      "-hide_banner", "-loglevel", "error",
      "-f", "lavfi", "-i", "testsrc=size=320x180:rate=25:duration=4",
      "-f", "lavfi", "-i", "sine=frequency=440:duration=1",
      "-f", "lavfi", "-i", "anullsrc=r=44100:cl=mono:d=2",
      "-f", "lavfi", "-i", "sine=frequency=440:duration=1",
      "-filter_complex", "[1:a][2:a][3:a]concat=n=3:v=0:a=1[a]",
      "-map", "0:v", "-map", "[a]", "-shortest", file,
    ]);
    const frames = await tool<{ frames: { ok: boolean }[] }>("view_frames", { file, seconds: [0.5, 3] });
    expect(frames.json.frames.map((frame) => frame.ok)).toEqual([true, true]);
    expect(frames.images).toHaveLength(2);

    const silences = await tool<{ duration: number; silences: [number, number][]; speech: [number, number][] }[]>("find_silences", { files: [file, path.join(dir, "gone.wav")] });
    const [found, missing] = silences.json;
    expect(found!.silences).toHaveLength(1);
    const [start, end] = found!.silences[0]!;
    expect(start).toBeCloseTo(1.1, 1);
    expect(end).toBeCloseTo(2.9, 1);
    expect(found!.speech).toHaveLength(2);
    expect(missing).toMatchObject({ error: expect.stringContaining("There is no file") });
  });

  it("names, switches and locks tracks of AI timelines only, adding missing ones", async () => {
    const { tool, calls } = setup();
    expect(await refusal(tool("set_tracks", { tracks: [{ type: "video", index: 1, name: "A" }] }))).toMatch(/isn't an AI timeline/);
    const result = await tool("set_tracks", {
      timeline: "AI - Cut",
      tracks: [{ type: "video", index: 2, name: "Titles", locked: true }, { type: "subtitle", index: 1, locked: true }],
    });
    expect(result.json).toMatchObject({ refused: ["subtitle 1: locked"] });
    expect(calls()).toEqual([
      ["AddTrack", "AI - Cut", "video"],
      ["SetTrackName", "video", 2, "Titles"],
      ["SetTrackLock", "video", 2, true],
      ["AddTrack", "AI - Cut", "subtitle"],
      ["SetTrackLock", "subtitle", 1, true],
    ]);
  });

  it("removes markers by frame and by color", async () => {
    const { tool } = setup();
    expect((await tool("remove_markers", { timeline: "AI - Cut", frames: [10, 99], color: "Red" })).json).toEqual({
      timeline: "AI - Cut",
      removed: 3,
      no_marker_at: [99],
    });
  });

  it("adds a Text+ title with its text at a frame", async () => {
    const { tool, calls } = setup();
    const result = await tool("add_title", { timeline: "AI - Cut", text: "Hello", frame: 90100, size: 0.1 });
    expect(result.json).toMatchObject({ id: "title-1", track: 2, start: 90100, end: 90225 });
    expect(calls()).toEqual([
      ["SetCurrentTimeline", "AI - Cut"],
      ["SetCurrentTimecode", "AI - Cut", "01:00:04:00"],
      ["InsertFusionTitleIntoTimeline", "AI - Cut", "Text+", "01:00:04:00"],
      ["SetInput", "title-1", "StyledText", "Hello"],
      ["SetInput", "title-1", "Size", 0.1],
    ]);
    expect(await refusal(tool("add_title", { timeline: "AI - Cut", text: "Hi", title: "Nope" }))).toMatch(/couldn't insert the title "Nope"/);
  });

  it("grades clips with a LUT, a CDL, or another clip's grade", async () => {
    const { tool, calls } = setup();
    const result = await tool("set_color", {
      timeline: "AI - Cut",
      item_ids: ["i3"],
      copy_from: "i2",
      lut: "Film.3dl",
      cdl: { slope: 1.1, offset: [0, 0, 0.01], saturation: 0.9 },
    });
    expect(result.json).toMatchObject({ refused: ["i3: lut"] });
    expect(calls()).toEqual([
      ["CopyGrades", "i2", ["i3"]],
      ["SetLUT", "i3", 1, "Film.3dl"],
      ["SetCDL", "i3", { NodeIndex: 1, Slope: "1.1 1.1 1.1", Offset: "0.0 0.0 0.01", Power: "1 1 1", Saturation: 0.9 }],
    ]);
  });

  it("files media into a bin and labels it, without deleting", async () => {
    const { tool, calls } = setup();
    const result = await tool("organize_media", { media_ids: ["m1"], bin: "Selects/Best", clip_color: "Rainbow", flag: "Green", metadata: { Comments: "Good" } });
    expect(result.json).toMatchObject({ clips: 1, moved_to: "Best", refused: ["m1: clip_color"] });
    expect(calls()).toEqual([
      ["AddSubFolder", "Master", "Selects"],
      ["AddSubFolder", "Selects", "Best"],
      ["MoveClips", ["m1"], "Best"],
      ["SetClipColor", "m1", "Rainbow"],
      ["AddFlag", "m1", "Green"],
      ["SetMetadata", "m1", { Comments: "Good" }],
    ]);
  });

  it("lists render options", async () => {
    const { tool } = setup();
    expect((await tool("render_options", { format: "mp4" })).json).toMatchObject({
      presets: ["H.264 Master", "YouTube - 1080p"],
      codecs: { format: "mp4", codecs: { "H.264": "H264" } },
    });
  });

  it("renders into the working folder, named after the timeline, and puts Resolve back", async () => {
    const { tool, project, calls } = setup();
    const result = await tool<{ file: string; timeline: string }>("render_timeline", { timeline: "AI - Cut", width: 1080, height: 1920, from_frame: 90000, to_frame: 90024 });
    expect(result.json).toMatchObject({ file: path.join(project, "renders", "AI - Cut.mp4"), timeline: "AI - Cut" });
    expect(fs.readFileSync(result.json.file, "utf8")).toBe("rendered");
    const log = calls();
    expect(log).toContainEqual(["SetCurrentRenderFormatAndCodec", "mp4", "H264"]);
    expect(log).toContainEqual([
      "SetRenderSettings",
      { TargetDir: path.join(project, "renders"), CustomName: "AI - Cut", SelectAllFrames: false, MarkIn: 90000, MarkOut: 90024, ExportVideo: true, ExportAudio: true, FormatWidth: 1080, FormatHeight: 1920 },
    ]);
    // Back to the timeline and render format that were set before.
    expect(log.slice(-4)).toEqual([
      ["DeleteRenderJob", "job-1"],
      ["LoadRenderPreset", "Linker-Edit restore"],
      ["SetCurrentRenderFormatAndCodec", "mov", "ProRes422HQ"],
      ["SetCurrentTimeline", "Edit"],
    ]);
  });

  it("renders audio only, and with a preset", async () => {
    const { tool, calls } = setup();
    const audio = await tool<{ file: string }>("render_timeline", { timeline: "t2", audio_only: true, name: "mix" });
    expect(path.basename(audio.json.file)).toBe("mix.wav");
    expect(calls()).toContainEqual(["SetCurrentRenderFormatAndCodec", "wav", "LinearPCM"]);
    expect(calls().find((call) => call[0] === "SetRenderSettings")![1]).toMatchObject({ ExportVideo: false, ExportAudio: true });
    expect(await refusal(tool("render_timeline", { preset: "Nope" }))).toMatch(/no render preset "Nope"/);
    expect(await refusal(tool("render_timeline", { format: "mp4", codec: "ProRes" }))).toMatch(/no codec "ProRes" for mp4/);
  });

  it("detects scene cuts on AI timelines only", async () => {
    const { tool } = setup();
    expect(await refusal(tool("detect_scene_cuts", {}))).toMatch(/isn't an AI timeline/);
    expect((await tool("detect_scene_cuts", { timeline: "AI - Cut" })).json).toMatchObject({ timeline: "AI - Cut", clips_before: 2, clips_after: 2 });
  });
});
