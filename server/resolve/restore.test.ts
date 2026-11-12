// restore_timeline over a real edit history, with Resolve's side stood in for: which scripts it runs, with what.
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { createHistory } from "../history/history.ts";
import type { RawSnapshot } from "../history/model.ts";
import { ResolveError, type ResolveConnection } from "./connection.ts";
import { runResolveTool } from "./run.ts";
import { readOrigins } from "./versions.ts";

const item = (id: string, start: number, fields: Record<string, unknown> = {}) => ({
  id,
  name: `clip ${id}`,
  start,
  end: start + 48,
  source_start: 10,
  source_end: 58,
  media: { id: `m-${id}`, name: `${id}.mov`, path: `D:/footage/${id}.mov` },
  enabled: true,
  properties: { ZoomX: 1 },
  ...fields,
});

const SNAPSHOT: RawSnapshot = {
  project: "Doc",
  project_id: "doc-id",
  current_timeline: "Interview",
  timelines: [
    {
      id: "t1",
      name: "Interview",
      frame_rate: "25",
      width: "1080",
      height: "1920",
      start_frame: 90000,
      end_frame: 90144,
      start_timecode: "01:00:00:00",
      video: [
        { index: 1, name: "Video 1", items: [item("a", 90000), item("b", 90048, { properties: { ZoomX: 1.5 } }), item("gone", 90096)] },
        { index: 2, name: "Video 2", items: [item("title", 90000, { media: null, name: "Text+" }), item("c", 90048, { enabled: false })] },
      ],
      audio: [{ index: 1, name: "Audio 1", items: [item("a", 90000)] }],
      markers: [{ frame: 24, color: "Red", name: "Look", note: "", duration: 1 }],
    },
  ],
};

type Clip = { media: string; track: number; start: number; type: string };

function fakeResolve(timelines: { id: string }[]) {
  const calls: [string, Record<string, unknown>][] = [];
  const connection: ResolveConnection = {
    command: "ResolveMCP",
    callTool: async () => "",
    close() {},
    runScript: async <T>(name: string, args?: unknown): Promise<T> => {
      const input = args as Record<string, unknown>;
      calls.push([name, input]);
      const results: Record<string, () => unknown> = {
        tool_list_timelines: () => timelines,
        tool_create_timeline: () => ({
          id: "new",
          name: `AI - ${String(input.name)}`,
          ...(input.replaces ? { replaces: { id: input.replaces, name: "Interview" } } : {}),
        }),
        // Media that left the media pool doesn't go on.
        tool_add_clips: () => ({
          added: (input.clips as Clip[])
            .filter((clip) => clip.media !== "m-gone")
            .map((clip, n) => ({ id: `added-${n}`, type: clip.type, track: clip.track, start: clip.start })),
        }),
        tool_set_clip_properties: () => ({}),
        tool_add_markers: () => ({}),
      };
      const result = results[name];
      if (!result) throw new ResolveError("error", `No ${name} in this test.`);
      return result() as T;
    },
  };
  return { connection, calls };
}

async function savedHistory() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-restore-"));
  const saved = await createHistory({ dir, readSnapshot: async () => SNAPSHOT }).record();
  return { dir, commit: saved!.commit };
}

describe("restore_timeline", () => {
  it("rebuilds a saved timeline as a new AI timeline: settings, clips on their tracks, properties and markers", async () => {
    const { dir, commit } = await savedHistory();
    const { connection, calls } = fakeResolve([{ id: "t1" }]);
    const result = JSON.parse((await runResolveTool(connection, "restore_timeline", { commit, timeline: "Interview" }, { dir })).text);

    expect(calls.map(([name]) => name)).toEqual([
      "tool_list_timelines",
      "tool_create_timeline",
      "tool_add_clips",
      "tool_set_clip_properties",
      "tool_set_clip_properties",
      "tool_add_markers",
    ]);
    expect(calls[1]![1]).toEqual({
      name: `Interview (restored ${commit.slice(0, 7)})`,
      width: 1080,
      height: 1920,
      frame_rate: 25,
      start_timecode: "01:00:00:00",
      replaces: "t1",
    });
    const clips = calls[2]![1].clips as (Clip & { source_start: number })[];
    expect(clips.map(({ media, track, start, type }) => `${type} ${track} ${start} ${media}`)).toEqual([
      "video 1 90000 m-a",
      "video 1 90048 m-b",
      "video 1 90096 m-gone",
      "video 2 90048 m-c",
      "audio 1 90000 m-a",
    ]);
    expect(clips[0]!.source_start).toBe(10);
    expect(calls.slice(3, 5).map(([, input]) => input)).toEqual([
      { timeline: "new", item_ids: ["added-1"], properties: { ZoomX: 1.5 } },
      { timeline: "new", item_ids: ["added-2"], properties: {}, enabled: false },
    ]);
    expect(calls[5]![1]).toEqual({ timeline: "new", markers: [{ frame: 24, color: "Red", name: "Look", note: "", duration: 1 }] });

    expect(result).toMatchObject({
      timeline: `AI - Interview (restored ${commit.slice(0, 7)})`,
      replaces: "t1",
      clips: { saved: 6, placed: 4 },
      properties_set: 2,
      markers: 1,
      left_out: [expect.stringContaining("Text+"), expect.stringContaining("clip gone")],
    });
    // The restored timeline can become the main one once the user approves it.
    expect(readOrigins(dir)).toEqual({ new: { id: "t1", name: "Interview" } });
  });

  it("restores a timeline that's gone from Resolve without replacing anything, and says when a save has no such timeline", async () => {
    const { dir, commit } = await savedHistory();
    const { connection, calls } = fakeResolve([]);
    await runResolveTool(connection, "restore_timeline", { commit, timeline: "t1" }, { dir });
    expect(calls[1]![1]).not.toHaveProperty("replaces");
    expect(readOrigins(dir)).toEqual({});

    await expect(runResolveTool(connection, "restore_timeline", { commit, timeline: "Promo" }, { dir })).rejects.toThrow(
      `The save ${commit} has no timeline "Promo"`,
    );
    await expect(runResolveTool(connection, "restore_timeline", { commit: "--output=x", timeline: "t1" }, { dir })).rejects.toThrow("has no timeline");
  });
});
