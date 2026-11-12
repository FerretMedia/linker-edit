import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { shortenText } from "../providers/shorten.ts";
import { findFfmpeg } from "../transcribe/tools.ts";
import { ResolveError, type ResolveConnection } from "./connection.ts";
import { findSilences, framesDir, mediaFrame, shrinkImage } from "./media.ts";
import { readSavedTimeline } from "../history/history.ts";
import { renderTimeline } from "./render.ts";
import { findResolveTool } from "./tools.ts";
import { recordOrigin, type TimelineRef } from "./versions.ts";

export interface ToolContext {
  /** The project's working folder, for exports and renders. */
  dir: string;
  /** Where frames for view_frames go; temporary. */
  framesRoot?: string;
  /** The ffmpeg to use; found when left out. */
  ffmpeg?: () => string;
  /** How often long jobs are checked on. */
  pollMs?: number;
  /** Aborts when the user stops the turn: long jobs such as renders stop with it. */
  signal?: AbortSignal;
}

export interface ToolResult {
  text: string;
  /** Image files for the agent to see, sent with the text. */
  images?: string[];
}

type Input = Record<string, unknown>;
type Handler = (connection: ResolveConnection, input: Input, context: ToolContext) => Promise<unknown>;

const MAX_FRAMES = 12;
const EXTENSIONS: Record<string, string> = { otio: "otio", edl: "edl", fcpxml: "fcpxml", fcp7xml: "xml", aaf: "aaf", csv: "csv", drt: "drt" };
const IMPORTABLE = new Set([".otio", ".edl", ".fcpxml", ".xml", ".aaf"]);

const text = (value: unknown) => (typeof value === "string" && value.trim() ? value : undefined);
const num = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : undefined);
const numbers = (value: unknown) => (Array.isArray(value) ? value.filter((item): item is number => typeof item === "number" && Number.isFinite(item)) : []);
const strings = (value: unknown) => (Array.isArray(value) ? value.filter((item): item is string => typeof item === "string") : []);
const safeName = (name: string) => name.replace(/[\\/:*?"<>|]+/g, "-").trim() || "timeline";
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));
const ffmpegOf = (context: ToolContext) => (context.ffmpeg ?? findFfmpeg)();

function existingFile(file: string, base: string): string {
  const full = path.resolve(base, file);
  if (!fs.existsSync(full) || !fs.statSync(full).isFile()) throw new Error(`There is no file ${file} on this computer.`);
  return full;
}

/** Tools that do part or all of their work in Node, around Resolve's scripts. */
const HANDLERS: Record<string, Handler> = {
  // Both remember what the new AI timeline is a version of, so the user can make it the main one later.
  async duplicate_timeline(connection, input, context) {
    const result = await connection.runScript<{ id: string; copied_from: TimelineRef }>("tool_duplicate_timeline", input, 30);
    recordOrigin(context.dir, result.id, result.copied_from);
    return result;
  },

  async create_timeline(connection, input, context) {
    const result = await connection.runScript<{ id: string; replaces?: TimelineRef }>("tool_create_timeline", input, 30);
    if (result.replaces) recordOrigin(context.dir, result.id, result.replaces);
    return result;
  },

  async restore_timeline(connection, input, context) {
    const commit = text(input.commit);
    const ref = text(input.timeline);
    if (!commit || !ref) throw new Error("Pass the save's commit and the timeline's id or exact name in that save.");
    const saved = await readSavedTimeline(context.dir, commit.trim(), ref);
    if (!saved) {
      throw new Error(`The save ${commit} has no timeline "${ref}". git log -- resolve lists the saves, and git show --stat <commit> their timelines.`);
    }
    const { metadata, cuts, audio, markers } = saved.timeline;

    // The saved timeline, if it's still there: the restored one is a new version of it.
    const existing = await connection.runScript<{ id: string }[]>("tool_list_timelines", {}, 30);
    const original = existing.some((timeline) => timeline.id === metadata.id) ? metadata.id : null;
    const created = (await HANDLERS.create_timeline!(
      connection,
      {
        name: `${metadata.name.replace(/^AI - /, "")} (restored ${commit.trim().slice(0, 7)})`,
        width: metadata.width,
        height: metadata.height,
        frame_rate: metadata.frame_rate,
        start_timecode: metadata.start_timecode,
        ...(original ? { replaces: original } : {}),
      },
      context,
    )) as { id: string; name: string };

    const rows = [...cuts.map((clip) => ({ clip, type: "video" as const })), ...audio.map((clip) => ({ clip, type: "audio" as const }))];
    const placeable = rows.filter(({ clip }) => clip.media);
    const leftOut = rows.filter(({ clip }) => !clip.media).map(({ clip, type }) => `${clip.name} (${type} track ${clip.track}, a title or generator)`);
    type Added = { id: string; type: string | null; track: number | null; start: number | null };
    const added: Added[] = [];
    const key = (type: string | null, track: number | null, start: number | null) => `${type}:${track}:${start}`;
    // A few hundred clips per script, so a long timeline stays inside Resolve's 60 seconds.
    for (let at = 0; at < placeable.length; at += 200) {
      const chunk = placeable.slice(at, at + 200);
      try {
        const result = await connection.runScript<{ added: Added[] }>(
          "tool_add_clips",
          {
            timeline: created.id,
            clips: chunk.map(({ clip, type }) => ({
              media: clip.media,
              source_start: clip.source_start,
              source_end: clip.source_end,
              start: clip.start,
              track: clip.track,
              type,
            })),
          },
          60,
        );
        added.push(...result.added);
      } catch (error) {
        if (!(error instanceof ResolveError && error.kind === "refused")) throw error;
      }
    }
    const placed = new Map(added.map((item) => [key(item.type, item.track, item.start), item.id]));
    const missing = placeable
      .filter(({ clip, type }) => !placed.has(key(type, clip.track, clip.start)))
      .map(({ clip, type }) => `${clip.name} (${type} track ${clip.track} at frame ${clip.start}: its media isn't in the media pool any more)`);

    // Clip properties and disabled clips, one call per distinct set of values.
    const groups = new Map<string, { properties: Record<string, unknown>; enabled?: boolean; ids: string[] }>();
    for (const { clip, type } of placeable) {
      const id = placed.get(key(type, clip.track, clip.start));
      if (!id || (!clip.properties && clip.enabled !== false)) continue;
      const change = { properties: clip.properties ?? {}, ...(clip.enabled === false ? { enabled: false } : {}) };
      const group = groups.get(JSON.stringify(change)) ?? { ...change, ids: [] };
      group.ids.push(id);
      groups.set(JSON.stringify(change), group);
    }
    const refused: string[] = [];
    for (const { ids, ...change } of groups.values()) {
      const result = await connection.runScript<{ refused?: string[] }>("tool_set_clip_properties", { timeline: created.id, item_ids: ids, ...change }, 30);
      refused.push(...(result.refused ?? []));
    }
    if (markers.length) await connection.runScript("tool_add_markers", { timeline: created.id, markers }, 30);

    return {
      timeline: created.name,
      id: created.id,
      replaces: original,
      clips: { saved: rows.length, placed: added.length },
      properties_set: [...groups.values()].reduce((sum, group) => sum + group.ids.length, 0),
      markers: markers.length,
      ...(leftOut.length || missing.length ? { left_out: [...leftOut, ...missing] } : {}),
      ...(refused.length ? { refused_properties: refused } : {}),
      note: "Color grades, effects, titles and generators aren't in the edit history, so they aren't restored. Read the timeline back with get_timeline.",
    };
  },

  async import_media(connection, input) {
    const paths = strings(input.paths);
    if (paths.length === 0) throw new Error("Pass the files to import in paths.");
    const bad = paths.filter((file) => !path.isAbsolute(file) || !fs.existsSync(file));
    if (bad.length) throw new Error(`These aren't files on this computer: ${bad.join(", ")}. Pass absolute paths.`);
    return connection.runScript("tool_import_media", input, 60);
  },

  async find_silences(_connection, input, context) {
    const files = strings(input.files);
    if (files.length === 0) throw new Error("Pass at least one file path in files.");
    const ffmpeg = ffmpegOf(context);
    const options = { minSilence: num(input.min_silence) ?? 0.5, thresholdDb: num(input.threshold_db) ?? -35, padding: num(input.padding) ?? 0.1 };
    return Promise.all(
      files.map(async (file) => {
        try {
          return await findSilences(ffmpeg, existingFile(file, context.dir), options);
        } catch (error) {
          return { file, error: error instanceof Error ? error.message : String(error) };
        }
      }),
    );
  },

  async view_frames(connection, input, context) {
    const root = context.framesRoot ?? path.join(os.tmpdir(), "linker-edit-frames");
    const file = text(input.file);
    if (file) {
      const seconds = numbers(input.seconds).slice(0, MAX_FRAMES);
      if (seconds.length === 0) throw new Error("Pass the moments to see in seconds.");
      const ffmpeg = ffmpegOf(context);
      const source = existingFile(file, context.dir);
      const dir = framesDir(root);
      const frames = await Promise.all(
        seconds.map(async (second, index) => {
          const out = path.join(dir, `frame-${index}.jpg`);
          return { seconds: second, file: out, ok: await mediaFrame(ffmpeg, source, second, out) };
        }),
      );
      return { result: { file: source, frames }, images: frames.filter((frame) => frame.ok).map((frame) => frame.file) };
    }
    const frames = numbers(input.frames).slice(0, MAX_FRAMES);
    if (frames.length === 0) throw new Error("Pass timeline frames, or a file with seconds.");
    const dir = framesDir(root);
    const stills = await connection.runScript<{ timeline: string; stills: { frame: number; file: string; ok: boolean }[] }>(
      "timeline_stills",
      { timeline: input.timeline, frames, dir: dir.replaceAll("\\", "/") },
      60,
    );
    let ffmpeg: string | null = null;
    try {
      ffmpeg = ffmpegOf(context);
    } catch {
      // Without ffmpeg the agent gets Resolve's full-size stills.
    }
    const shown = [];
    for (const still of stills.stills) {
      const exported = still.ok && fs.existsSync(still.file);
      shown.push({ frame: still.frame, ok: exported, file: exported ? await shrinkImage(ffmpeg, still.file) : null });
    }
    const missing = shown.some((still) => !still.ok);
    return {
      result: {
        timeline: stills.timeline,
        frames: shown,
        ...(missing ? { note: "Resolve exported no still for some frames: outside the timeline, or Resolve is busy." } : {}),
      },
      images: shown.flatMap((still) => (still.file ? [still.file] : [])),
    };
  },

  async export_timeline(connection, input, context) {
    const format = text(input.format) ?? "otio";
    const extension = EXTENSIONS[format];
    if (!extension) throw new Error(`Export formats are ${Object.keys(EXTENSIONS).join(", ")}.`);
    const dir = path.join(context.dir, "exports");
    fs.mkdirSync(dir, { recursive: true });
    const name = text(input.name);
    const result = await connection.runScript<{ path: string }>(
      "tool_export_timeline",
      { timeline: input.timeline, format, dir: dir.replaceAll("\\", "/"), extension, ...(name ? { name: safeName(name) } : {}) },
      60,
    );
    const file = path.resolve(result.path);
    if (!fs.existsSync(file)) throw new Error("Resolve said it exported the timeline, but the file isn't there.");
    return { ...result, path: file, relative: path.relative(context.dir, file).replaceAll("\\", "/") };
  },

  async import_timeline(connection, input, context) {
    const file = existingFile(text(input.path) ?? "", context.dir);
    if (!IMPORTABLE.has(path.extname(file).toLowerCase())) throw new Error("import_timeline reads .otio, .edl, .fcpxml, .xml and .aaf files.");
    return connection.runScript("tool_import_timeline", { ...input, path: file }, 60);
  },

  async detect_scene_cuts(connection, input, context) {
    type Status = { status: { JobStatus?: string; CompletionPercentage?: number }; clips: number; name: string };
    let started: { timeline_id: string; before: number } | null = null;
    try {
      started = await connection.runScript("scene_cuts_start", input, 60);
    } catch (error) {
      // Resolve may still be cutting when the script runs out of time; the status below says.
      if (!(error instanceof ResolveError && error.kind === "timeout")) throw error;
    }
    const timeline = started?.timeline_id ?? input.timeline;
    const deadline = Date.now() + 15 * 60_000;
    let status: Status;
    for (;;) {
      status = await connection.runScript<Status>("scene_cuts_status", { timeline }, 10);
      const job = status.status.JobStatus ?? "";
      if (!/^(Initializing|Analyzing|Processing|Running|InProgress|In Progress|Cutting)$/i.test(job) || Date.now() > deadline) break;
      // Resolve finishes the cut by itself; a stopped turn only stops waiting for it.
      if (context.signal?.aborted) throw new Error("Stopped by the user while Resolve was still detecting scene cuts.");
      await sleep(context.pollMs ?? 2000);
    }
    return { timeline: status.name, clips_before: started?.before ?? null, clips_after: status.clips, status: status.status };
  },

  async render_timeline(connection, input, context) {
    const audioOnly = input.audio_only === true;
    const dir = text(input.dir) ?? path.join(context.dir, "renders");
    if (!path.isAbsolute(dir)) throw new Error("dir must be an absolute folder.");
    const job = renderTimeline(
      connection,
      {
        timelineId: text(input.timeline) ?? null,
        dir,
        name: text(input.name),
        preset: text(input.preset),
        format: text(input.format) ?? (audioOnly ? "wav" : undefined),
        codec: text(input.codec),
        width: num(input.width),
        height: num(input.height),
        fromFrame: num(input.from_frame),
        toFrame: num(input.to_frame),
        ...(audioOnly ? { audio: true, video: false } : {}),
      },
      () => {},
      context.pollMs ?? 1000,
    );
    // An hour at most, so a stuck render doesn't hold Resolve forever; stopping the turn stops it too.
    const timer = setTimeout(job.stop, 60 * 60_000);
    if (context.signal?.aborted) job.stop();
    context.signal?.addEventListener("abort", job.stop, { once: true });
    const started = Date.now();
    const result = await job.finished.finally(() => {
      clearTimeout(timer);
      context.signal?.removeEventListener("abort", job.stop);
    });
    if (result.event === "error" && result.stopped && context.signal?.aborted) throw new Error("Render stopped by the user.");
    if (result.event !== "done") throw new Error(result.event === "error" ? result.message : "The render didn't finish.");
    return { file: result.file, timeline: result.timeline.name, seconds: Math.round((Date.now() - started) / 1000) };
  },
};

/** Tools whose work starts in Node rather than in a tool_<name>.py script of their own. */
export const NODE_TOOLS = new Set(Object.keys(HANDLERS));

/** Runs one of Linker-Edit's Resolve tools. Its result is compact JSON, shortened when long. */
export async function runResolveTool(connection: ResolveConnection, name: string, input: Input, context: ToolContext): Promise<ToolResult> {
  const tool = findResolveTool(name);
  if (!tool) throw new Error(`There is no Resolve tool called ${name}.`);
  const handler = HANDLERS[name];
  const output = handler ? await handler(connection, input, context) : await connection.runScript(`tool_${name}`, input, tool.timeout ?? 30);
  const withImages = output && typeof output === "object" && "images" in output && "result" in output ? (output as { result: unknown; images: string[] }) : null;
  const json = JSON.stringify(withImages ? withImages.result : output);
  return { text: shortenText(json) ?? json, ...(withImages?.images.length ? { images: withImages.images } : {}) };
}
