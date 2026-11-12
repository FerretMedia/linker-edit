// Linker-Edit's Resolve scripts, run in this computer's Python against a stand-in Resolve
// (fixtures/DaVinciResolveScript.py), the way Resolve's run_script runs them.
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { readResolveSnapshot } from "../history/history.ts";
import { ResolveError } from "./connection.ts";
import { hasPython, pythonConnection } from "./testing.ts";
import { NODE_TOOLS } from "./run.ts";
import { RESOLVE_TOOLS } from "./tools.ts";

const FAKE = fs.readFileSync(path.join(import.meta.dirname, "fixtures", "DaVinciResolveScript.py"), "utf8");

const clip = (start: number, end: number, id: string, media = "m1") => ({ id, name: `${media}.mov`, start, end, source_start: 100, source_end: 100 + end - start, media });

const STATE = {
  project: {
    name: "Doc",
    id: "doc-id",
    current: "Edit",
    timelines: [
      { id: "t1", name: "Edit", video: [[clip(90000, 90050, "i1"), clip(90050, 90100, "i2", "m2")]], audio: [[clip(90000, 90100, "i3")]] },
      { id: "t2", name: "AI - Cut", video: [[clip(90000, 90025, "i4")]], audio: [], markers: [{ frame: 10, color: "Blue", name: "", note: "", duration: 1 }] },
    ],
  },
  media: {
    name: "Master",
    clips: [{ id: "m1", name: "m1.mov", path: "D:/Footage/m1.mov", props: { FPS: 25, Type: "Video" } }],
    folders: [{ name: "Interviews", clips: [{ id: "m2", name: "m2.mov", path: "D:/Footage/Interviews/m2.mov", props: { FPS: 25 } }] }],
  },
};

function resolve(state: object = STATE) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-scripts-"));
  const file = path.join(dir, "state.json");
  fs.writeFileSync(file, JSON.stringify(state));
  const connection = pythonConnection(FAKE, { FAKE_RESOLVE: file });
  const calls = () =>
    fs.existsSync(`${file}.log`)
      ? fs.readFileSync(`${file}.log`, "utf8").trim().split("\n").map((line) => JSON.parse(line) as unknown[])
      : [];
  const tool = <T = Record<string, unknown>>(name: string, input: object = {}) => connection.runScript<T>(`tool_${name}`, input);
  return { connection, calls, tool };
}

const refusal = (promise: Promise<unknown>) =>
  promise.then(
    () => null,
    (error: unknown) => (error instanceof ResolveError ? `${error.kind}: ${error.message}` : String(error)),
  );

describe.skipIf(!hasPython)("Resolve scripts", () => {
  it("has a script or a Node handler for every tool", () => {
    for (const tool of RESOLVE_TOOLS) {
      const script = fs.existsSync(path.join(import.meta.dirname, "scripts", `tool_${tool.name}.py`));
      expect(script || NODE_TOOLS.has(tool.name), tool.name).toBe(true);
    }
  });

  it("reads the status", async () => {
    const { connection } = resolve();
    expect(await connection.runScript("status")).toEqual({
      state: "connected",
      product: "DaVinci Resolve Studio",
      version: "21.1.2",
      project: "Doc",
      projectId: "doc-id",
      database: { type: "Disk", name: "Local Database" },
      timeline: "Edit",
      page: "edit",
    });
  });

  it("lists timelines, marking the current one and the AI ones", async () => {
    const timelines = await resolve().tool<{ name: string; current: boolean; ai: boolean }[]>("list_timelines");
    expect(timelines.map(({ name, current, ai }) => ({ name, current, ai }))).toEqual([
      { name: "Edit", current: true, ai: false },
      { name: "AI - Cut", current: false, ai: true },
    ]);
  });

  it("reads a timeline's clips with their media, narrowed to a track type and frame range", async () => {
    const timeline = await resolve().tool<{ video: { items: { id: string }[] }[]; audio: unknown[]; media: object }>("get_timeline", {
      track_type: "video",
      from_frame: 90060,
    });
    expect(timeline.video[0]!.items).toEqual([{ id: "i2", name: "m2.mov", start: 90050, end: 90100, source_start: 100, source_end: 150, media: "m2" }]);
    expect(timeline.audio).toEqual([]);
    expect(timeline.media).toEqual({ m2: { name: "m2.mov", path: "D:/Footage/Interviews/m2.mov", fps: 25 } });
  });

  it("lists media with bins, filtered by bin", async () => {
    const media = await resolve().tool<{ bins: string[]; clips: { id: string; bin: string; file_path: string }[] }>("list_media", { bin: "Interviews" });
    expect(media.bins).toEqual(["Master", "Master/Interviews"]);
    expect(media.clips).toEqual([expect.objectContaining({ id: "m2", bin: "Master/Interviews", file_path: "D:/Footage/Interviews/m2.mov" })]);
  });

  it("names new timelines AI - …, unique and without slashes, and opens them", async () => {
    const { tool, calls } = resolve({ ...STATE, project: { ...STATE.project, timelines: [...STATE.project.timelines, { id: "t3", name: "AI - Cut 2" }] } });
    expect(await tool("create_timeline", { name: "AI - Cut", width: 1080, height: 1920, frame_rate: 30 })).toMatchObject({
      name: "AI - Cut 3",
      refused_settings: ["timelineFrameRate"],
    });
    expect(calls()).toContainEqual(["CreateEmptyTimeline", "AI - Cut 3"]);
    expect(calls().at(-1)).toEqual(["SetCurrentTimeline", "AI - Cut 3"]);
    expect(calls()).toContainEqual(["SetSetting", "AI - Cut 3", "timelineResolutionWidth", "1080"]);
    expect(await refusal(resolve().tool("create_timeline", { name: " / " }))).toBe("refused: Give the timeline a name.");
  });

  it("copies any timeline to an AI one", async () => {
    const { tool, calls } = resolve();
    expect(await tool("duplicate_timeline", {})).toMatchObject({ name: "AI - Edit copy", copied_from: { id: "t1", name: "Edit" } });
    expect(calls()[0]).toEqual(["DuplicateTimeline", "Edit", "AI - Edit copy"]);
  });

  it("changes only AI timelines", async () => {
    const { tool, calls } = resolve();
    for (const [name, input] of [
      ["add_clips", { clips: [{ media: "m1" }] }],
      ["remove_clips", { item_ids: ["i1"] }],
      ["set_clip_properties", { item_ids: ["i1"], properties: { ZoomX: 2 } }],
      ["add_markers", { markers: [{ frame: 1 }] }],
    ] as const) {
      expect(await refusal(tool(name, input))).toMatch(/^refused: "Edit" isn't an AI timeline/);
    }
    expect(calls()).toEqual([]);
  });

  it("puts history rows back on a timeline as they are, adding missing tracks", async () => {
    const { tool, calls } = resolve();
    const result = await tool<{ added: { track: number; start: number }[] }>("add_clips", {
      timeline: "AI - Cut",
      clips: [
        { id: "old", name: "m1.mov", media: "m1", source_start: 10, source_end: 60, start: 90100, end: 90150, track: 2, type: "video" },
        { media: "m2", type: "audio" },
      ],
    });
    expect(result.added).toEqual([
      expect.objectContaining({ type: "video", track: 2, start: 90100, end: 90150 }),
      expect.objectContaining({ type: "audio", track: 1 }),
    ]);
    expect(calls()).toEqual([
      ["AddTrack", "AI - Cut", "video"],
      ["SetCurrentTimeline", "AI - Cut"],
      [
        "AppendToTimeline",
        "AI - Cut",
        [
          { mediaPoolItem: "m1", startFrame: 10, endFrame: 60, recordFrame: 90100, mediaType: 1, trackIndex: 2 },
          { mediaPoolItem: "m2", mediaType: 2 },
        ],
      ],
    ]);
  });

  it("says which media or clips don't exist", async () => {
    const { tool } = resolve();
    expect(await refusal(tool("add_clips", { timeline: "AI - Cut", clips: [{ media: "gone" }] }))).toMatch(/^refused: These media ids aren't in the media pool: gone/);
    expect(await refusal(tool("remove_clips", { timeline: "AI - Cut", item_ids: ["i1"] }))).toMatch(/^refused: These clips aren't on the timeline: i1/);
    expect(await refusal(tool("get_timeline", { timeline: "Nope" }))).toMatch(/^refused: There is no timeline "Nope"/);
  });

  it("sets properties and markers, reporting what Resolve refused", async () => {
    const { tool, calls } = resolve();
    expect(await tool("set_clip_properties", { timeline: "t2", item_ids: ["i4"], properties: { ZoomX: 1.2, Nonsense: 1 }, enabled: false })).toMatchObject({
      refused: ["i4: Nonsense"],
    });
    expect(await tool("add_markers", { timeline: "AI - Cut", markers: [{ frame: 10 }, { frame: 20, color: "Red", name: "Here" }] })).toMatchObject({
      added: 1,
      refused_frames: [10],
    });
    expect(calls()).toContainEqual(["SetClipEnabled", "i4", false]);
    expect(calls()).toContainEqual(["AddMarker", "AI - Cut", 20, "Red", "Here", "", 1]);
  });

  it("imports into a bin it makes, then goes back to the bin that was open", async () => {
    const { tool, calls } = resolve();
    expect(await tool("import_media", { paths: ["D:/Music/song.wav"], bin: "AI imports/Music" })).toEqual({
      bin: "Music",
      imported: [{ id: "imported-0", name: "song.wav", file_path: "D:/Music/song.wav" }],
    });
    expect(calls()).toEqual([
      ["AddSubFolder", "Master", "AI imports"],
      ["AddSubFolder", "AI imports", "Music"],
      ["SetCurrentFolder", "Music"],
      ["AddItemListToMediaPool", "Music", ["D:/Music/song.wav"]],
      ["SetCurrentFolder", "Master"],
    ]);
  });

  it("saves the history in pages when a project takes long to read", async () => {
    const { connection } = resolve();
    const pages: unknown[] = [];
    const snapshot = await readResolveSnapshot({
      ...connection,
      runScript: <T>(name: string, args?: unknown) => {
        // A budget of 0 reads one timeline per run.
        pages.push(args);
        return connection.runScript<T>(name, { ...(args as object), budget: 0 });
      },
    });
    expect(pages).toEqual([{ ids: null, current_only: false, budget: 40 }, { ids: ["t2"], budget: 40 }]);
    expect(snapshot).toMatchObject({ project: "Doc", project_id: "doc-id", current_timeline: "Edit" });
    expect(snapshot.timelines.map((timeline) => timeline.name)).toEqual(["Edit", "AI - Cut"]);
    expect(snapshot.timelines[0]!.video[0]!.items[0]).toMatchObject({ id: "i1", media: { id: "m1", path: "D:/Footage/m1.mov" } });
    expect(snapshot.timelines[1]!.markers).toEqual([{ frame: 10, color: "Blue", name: "", note: "", duration: 1 }]);
  });

  it("reads only the timeline the preview asks for", async () => {
    const { connection } = resolve();
    const names = async (only: string | null) => (await readResolveSnapshot(connection, { only })).timelines.map((timeline) => timeline.name);
    expect(await names(null)).toEqual(["Edit"]);
    expect(await names("t2")).toEqual(["AI - Cut"]);
  });
});
