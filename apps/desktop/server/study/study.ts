// The agents' study tools on the "linker" MCP server: measuring finished videos (study_media) and lining a
// finished video up with its raw footage (compare_cut). Neither needs Resolve.
import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { VIDEO } from "../library/scan.ts";
import type { TranscriptResult } from "../transcribe/transcriber.ts";
import type { Transcript } from "../transcribe/whisper.ts";
import { compareCut, tokens } from "./compare.ts";
import { measureVideo, STUDY_VERSION, type Study } from "./measure.ts";

export const STUDY_MEDIA = "study_media";
export const COMPARE_CUT = "compare_cut";

export const STUDY_TOOLS = [
  {
    name: STUDY_MEDIA,
    description:
      "Measure finished videos, to learn how they are edited: length, aspect ratio, frame rate, loudness and true peak, shot lengths and cuts per minute, how quiet it gets, words per minute, and how it opens and closes. Shows you a contact sheet of each: small frames across the whole video, left to right and top to bottom. Pass video files or folders of them, at most 8 videos per call. Results are kept until a file changes. It misses cuts between similar frames (jump cuts in a talking head), and it can't tell music from other sound.",
    inputSchema: {
      type: "object",
      properties: {
        files: { type: "array", items: { type: "string" }, description: "Absolute paths of video files, or of folders with videos in them." },
        transcribe: { type: "boolean", description: "Also transcribe each video, for words per minute and its opening and closing words. Defaults to true." },
      },
      required: ["files"],
    },
    annotations: { readOnlyHint: true },
  },
  {
    name: COMPARE_CUT,
    description:
      "Line a finished video up with the raw footage it was cut from, by what is said in both. Shows what the editor kept (with times in both), what they cut and whether that was a retake or content left out, which take they used when something was said more than once, whether they reordered, and how much silence they left at cuts. Needs speech in both; transcribes them first.",
    inputSchema: {
      type: "object",
      properties: {
        final: { type: "string", description: "Absolute path of the finished video." },
        raw: { type: "array", items: { type: "string" }, description: "Absolute paths of the raw footage files, or of folders with them." },
        language: { type: "string", description: 'Spoken language as a Whisper code such as "en" or "nl". Leave out to detect it per file.' },
      },
      required: ["final", "raw"],
    },
    annotations: { readOnlyHint: true },
  },
];

export const isStudyTool = (name: string) => name === STUDY_MEDIA || name === COMPARE_CUT;

const MAX_STUDIED = 8;
const MAX_RAW = 60;
const OPENING_WORDS = 40;
const CLOSING_WORDS = 25;

export interface StudyOptions {
  /** Where measurements and contact sheets are kept. */
  dir: string;
  ffmpeg: () => string;
  transcribe: (files: string[], language?: string) => Promise<TranscriptResult[]>;
  measure?: typeof measureVideo;
}

const strings = (value: unknown) => (Array.isArray(value) ? value.filter((item): item is string => typeof item === "string" && item.trim() !== "") : []);
const round = (value: number, digits = 2) => Math.round(value * 10 ** digits) / 10 ** digits;

/** The video files a list of files and folders stands for, folders read one level deep and in name order. */
export function videoFiles(paths: string[]): string[] {
  const files: string[] = [];
  for (const entry of paths) {
    const full = path.resolve(entry);
    let stat: fs.Stats;
    try {
      stat = fs.statSync(full);
    } catch {
      throw new Error(`There is no file or folder ${entry} on this computer.`);
    }
    if (stat.isFile()) files.push(full);
    else {
      const names = fs.readdirSync(full, { withFileTypes: true });
      const inside = names
        .filter((name) => name.isFile() && !name.name.startsWith(".") && VIDEO.has(path.extname(name.name).toLowerCase()))
        .map((name) => path.join(full, name.name))
        .sort((a, b) => a.localeCompare(b));
      if (inside.length === 0) throw new Error(`${entry} has no video files in it. Folders inside it aren't read: pass them too.`);
      files.push(...inside);
    }
  }
  return [...new Set(files)];
}

/** Words per minute of talk, and the first and last words, from a transcript. */
export function speechOf(transcript: Transcript) {
  const words = tokens(transcript);
  const talk = transcript.segments.reduce((sum, segment) => sum + Math.max(0, segment.end - segment.start), 0);
  const all = transcript.segments.flatMap((segment) => segment.words.map((word) => word.text));
  return {
    language: transcript.language,
    words: words.length,
    words_per_minute: talk > 0 ? Math.round((words.length / talk) * 60) : 0,
    /** How much of the video someone is talking, 0 to 1. */
    speech_share: transcript.duration > 0 ? round(Math.min(1, talk / transcript.duration)) : 0,
    first_word_at_seconds: words[0] ? round(words[0].start) : null,
    opens_with: all.slice(0, OPENING_WORDS).join(" "),
    ...(all.length > OPENING_WORDS + CLOSING_WORDS ? { closes_with: all.slice(-CLOSING_WORDS).join(" ") } : {}),
  };
}

export type StudyService = ReturnType<typeof createStudy>;

export function createStudy(options: StudyOptions) {
  // One at a time: each measurement decodes a whole video.
  let queue: Promise<unknown> = Promise.resolve();

  function cacheFiles(file: string) {
    const key = createHash("sha256").update(process.platform === "win32" ? file.toLowerCase() : file).digest("hex").slice(0, 12);
    const name = path.parse(file).name.replace(/[^\w.-]+/g, "_").slice(0, 60);
    const base = path.join(options.dir, `${name}-${key}`);
    return { json: `${base}.json`, sheet: `${base}.jpg` };
  }

  function cached(file: string): Study | null {
    try {
      const study = JSON.parse(fs.readFileSync(cacheFiles(file).json, "utf8")) as Study;
      const stat = fs.statSync(file);
      if (study.version !== STUDY_VERSION || study.size !== stat.size || study.modified !== stat.mtimeMs) return null;
      if (study.sheet && !fs.existsSync(study.sheet.file)) return null;
      return study;
    } catch {
      return null;
    }
  }

  function measure(file: string, signal?: AbortSignal): Promise<Study> {
    const hit = cached(file);
    if (hit) return Promise.resolve(hit);
    const job = queue.then(async () => {
      if (signal?.aborted) throw new Error("Stopped.");
      fs.mkdirSync(options.dir, { recursive: true });
      const files = cacheFiles(file);
      const study = await (options.measure ?? measureVideo)(options.ffmpeg(), file, files.sheet, signal);
      fs.writeFileSync(files.json, JSON.stringify(study));
      return study;
    });
    queue = job.catch(() => {});
    return job;
  }

  async function studyMedia(input: Record<string, unknown>, signal?: AbortSignal): Promise<{ text: string; images: string[] }> {
    const files = videoFiles(strings(input.files));
    if (files.length === 0) throw new Error("Pass video files or folders in files.");
    const now = files.slice(0, MAX_STUDIED);
    const transcripts = input.transcribe === false ? [] : await options.transcribe(now);
    const images: string[] = [];
    const videos = [];
    for (const [index, file] of now.entries()) {
      try {
        const { version: _version, size: _size, modified: _modified, sheet, ...study } = await measure(file, signal);
        const transcript = transcripts[index];
        if (sheet) images.push(sheet.file);
        videos.push({
          ...study,
          ...(transcript && "transcript" in transcript
            ? { speech: speechOf(transcript.transcript), transcript: transcript.textFile }
            : transcript
              ? { speech: null, transcript_problem: transcript.error }
              : {}),
          contact_sheet: sheet ? { image: images.length, frames: sheet.frames, step_seconds: sheet.step_seconds } : null,
        });
      } catch (error) {
        if (signal?.aborted) throw error;
        videos.push({ file, error: error instanceof Error ? error.message : String(error) });
      }
    }
    const left = files.slice(MAX_STUDIED);
    return {
      text: JSON.stringify({
        videos,
        note: "contact_sheet.image is which of the attached images shows that video, counted from 1. Frame n of a sheet (from 0) is at about n × step_seconds.",
        ...(left.length ? { not_studied_yet: left, more: `${left.length} more videos: call again with these.` } : {}),
      }),
      images,
    };
  }

  async function compare(input: Record<string, unknown>): Promise<{ text: string }> {
    const final = typeof input.final === "string" ? videoFiles([input.final]) : [];
    if (final.length !== 1) throw new Error("Pass the one finished video in final.");
    const raw = videoFiles(strings(input.raw)).filter((file) => file !== final[0]);
    if (raw.length === 0) throw new Error("Pass the raw footage files or folders in raw.");
    if (raw.length > MAX_RAW) throw new Error(`That is ${raw.length} raw files; pass at most ${MAX_RAW}, the ones this video was cut from.`);
    const language = typeof input.language === "string" ? input.language : undefined;
    const [finalResult, ...rawResults] = await options.transcribe([final[0]!, ...raw], language);
    if (!finalResult || "error" in finalResult) throw new Error(`Couldn't transcribe ${final[0]}: ${finalResult?.error ?? "no result"}`);
    const usable = rawResults.filter((result): result is Extract<TranscriptResult, { transcript: Transcript }> => "transcript" in result);
    const failed = rawResults.filter((result) => "error" in result).map((result) => result.file);
    if (usable.length === 0) throw new Error("None of the raw files could be transcribed.");
    if (tokens(finalResult.transcript).length === 0) throw new Error("Nobody speaks in the finished video, so there is nothing to line up.");
    const comparison = compareCut(
      finalResult.transcript,
      usable.map((result) => result.transcript),
    );
    return { text: JSON.stringify({ ...comparison, ...(failed.length ? { raw_not_transcribed: failed } : {}) }) };
  }

  return {
    /** Runs one of the study tools; signal stops a measurement when the turn is stopped. */
    run(tool: string, input: Record<string, unknown>, signal?: AbortSignal): Promise<{ text: string; images?: string[] }> {
      return tool === STUDY_MEDIA ? studyMedia(input, signal) : compare(input);
    },
  };
}
