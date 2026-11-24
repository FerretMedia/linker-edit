// Measures a video without a model: its size, shots, sound and loudness, and a contact sheet of its frames, with
// the ffmpeg Linker-Edit ships. This is what an agent studies a brand's example videos with.
import { execFile } from "node:child_process";
import * as fs from "node:fs";
import { parseSilences } from "../resolve/media.ts";

/** Bump when a Study changes shape or how it is measured, so cached ones are measured again. */
export const STUDY_VERSION = 2;

export interface Shots {
  count: number;
  average_seconds: number;
  median_seconds: number;
  longest_seconds: number;
  cuts_per_minute: number;
  /** Where the picture changes, in seconds; the first of them when there are very many. */
  cuts: number[];
}

export interface Sound {
  /** How much of the video is quiet, 0 to 1. Near 0 usually means music or ambience runs under everything. */
  quiet_share: number;
  /** Quiet stretches of 0.4 seconds or longer. */
  pauses: number;
  longest_pause_seconds: number;
  median_pause_seconds: number;
}

export interface ContactSheet {
  /** A JPEG of small frames, left to right and top to bottom. */
  file: string;
  frames: number;
  /** Frame n (from 0) is at about n × this many seconds. */
  step_seconds: number;
}

export interface Study {
  version: number;
  file: string;
  /** Size and modification time when it was measured, to spot a changed file. */
  size: number;
  modified: number;
  duration: number;
  width: number | null;
  height: number | null;
  /** "16:9", "9:16", "1:1"… */
  aspect: string | null;
  fps: number | null;
  loudness_lufs: number | null;
  /** The loudest moment, between samples too. Above 0 it distorts. */
  true_peak_db: number | null;
  /** Null when the picture couldn't be read. */
  shots: Shots | null;
  /** Null when the file has no audio. */
  sound: Sound | null;
  sheet: ContactSheet | null;
}

const round = (value: number, digits = 2) => Math.round(value * 10 ** digits) / 10 ** digits;

function run(ffmpeg: string, args: string[], signal?: AbortSignal): Promise<{ code: number; log: string }> {
  return new Promise((resolve) => {
    execFile(ffmpeg, args, { windowsHide: true, timeout: 30 * 60_000, maxBuffer: 256 * 1024 * 1024, signal }, (error, _stdout, stderr) =>
      resolve({ code: error ? ((error as { code?: number }).code ?? 1) : 0, log: String(stderr) }),
    );
  });
}

const COMMON_ASPECTS: [number, string][] = [
  [16 / 9, "16:9"],
  [9 / 16, "9:16"],
  [1, "1:1"],
  [4 / 5, "4:5"],
  [4 / 3, "4:3"],
  [3 / 4, "3:4"],
  [21 / 9, "21:9"],
  [2.39, "2.39:1"],
  [1.9, "1.9:1"],
];

export function aspectName(width: number, height: number): string {
  const ratio = width / height;
  const known = COMMON_ASPECTS.find(([value]) => Math.abs(value - ratio) / value < 0.02);
  return known ? known[1] : `${round(ratio)}:1`;
}

/** The picture's size as it is shown (phones record sideways and say so), frame rate and length, from ffmpeg's log. */
export function parseVideoInfo(log: string): Pick<Study, "duration" | "width" | "height" | "aspect" | "fps"> & { audio: boolean } {
  const input = log.split(/\nOutput #0|\nStream mapping:/)[0] ?? log;
  const time = /Duration: (\d+):(\d+):([\d.]+)/.exec(input);
  const duration = time ? Number(time[1]) * 3600 + Number(time[2]) * 60 + Number(time[3]) : 0;
  const video = /Stream #\d+:\d+[^\n]*: Video: [^\n]*?\b(\d{2,5})x(\d{2,5})\b([^\n]*)/.exec(input);
  let width = video ? Number(video[1]) : null;
  let height = video ? Number(video[2]) : null;
  const rotation = /rotation of (-?[\d.]+) degrees/.exec(input);
  if (width && height && rotation && Math.abs(Math.round(Number(rotation[1]) / 90)) % 2 === 1) [width, height] = [height, width];
  const rate = video ? /([\d.]+) fps/.exec(video[3] ?? "") : null;
  return {
    duration: round(duration, 3),
    width,
    height,
    aspect: width && height ? aspectName(width, height) : null,
    fps: rate ? Number(rate[1]) : null,
    audio: /Stream #\d+:\d+[^\n]*: Audio:/.test(input),
  };
}

const MAX_CUTS = 400;

/** Shot lengths from the times the picture changes (ffmpeg's showinfo after a scene filter). */
export function parseShots(log: string, duration: number): Shots | null {
  if (duration <= 0) return null;
  const cuts = [...log.matchAll(/pts_time:([\d.]+)/g)].map((match) => Number(match[1])).filter((time) => time > 0 && time < duration);
  const edges = [0, ...cuts, duration];
  const lengths = edges.slice(1).map((end, index) => end - edges[index]!).sort((a, b) => a - b);
  return {
    count: lengths.length,
    average_seconds: round(duration / lengths.length),
    median_seconds: round(lengths[Math.floor(lengths.length / 2)]!),
    longest_seconds: round(lengths.at(-1)!),
    cuts_per_minute: round((cuts.length / duration) * 60, 1),
    cuts: cuts.slice(0, MAX_CUTS).map((time) => round(time)),
  };
}

/** How quiet the video gets, from ffmpeg's silencedetect log. */
export function parseSound(log: string): Sound {
  const { duration, silences, silent_seconds } = parseSilences(log, 0);
  const lengths = silences.map(([start, end]) => end - start).sort((a, b) => a - b);
  return {
    quiet_share: duration > 0 ? round(silent_seconds / duration) : 0,
    pauses: lengths.length,
    longest_pause_seconds: round(lengths.at(-1) ?? 0),
    median_pause_seconds: round(lengths[Math.floor(lengths.length / 2)] ?? 0),
  };
}

/** The contact sheet's grid: wide videos get 6 across, tall ones 8, so both end up about as big. */
export function sheetLayout(width: number | null, height: number | null): { columns: number; rows: number; cellWidth: number; cellHeight: number } {
  const tall = !!width && !!height && height > width;
  const cellWidth = tall ? 180 : 320;
  const ratio = width && height ? height / width : 9 / 16;
  // Even, as JPEG encoders want.
  const cellHeight = Math.max(2, Math.round((cellWidth * ratio) / 2) * 2);
  return tall ? { columns: 8, rows: 3, cellWidth, cellHeight } : { columns: 6, rows: 4, cellWidth, cellHeight };
}

/** Measures one video. sheetFile is where its contact sheet goes. */
export async function measureVideo(ffmpeg: string, file: string, sheetFile: string, signal?: AbortSignal): Promise<Study> {
  const stat = fs.statSync(file);
  const header = await run(ffmpeg, ["-hide_banner", "-i", file], signal);
  const info = parseVideoInfo(header.log);
  if (info.duration <= 0) throw new Error(`ffmpeg couldn't read ${file}: ${header.log.trim().split("\n").at(-1) ?? ""}`);

  let sound: Sound | null = null;
  let loudness: number | null = null;
  let peak: number | null = null;
  if (info.audio) {
    const audio = await run(
      ffmpeg,
      ["-hide_banner", "-nostats", "-i", file, "-map", "0:a:0", "-af", "silencedetect=noise=-35dB:d=0.4,ebur128=framelog=verbose:peak=true", "-f", "null", "-"],
      signal,
    );
    sound = parseSound(audio.log);
    const integrated = /Integrated loudness:\s*\n\s*I:\s*(-?[\d.]+) LUFS/.exec(audio.log);
    const loudest = /True peak:\s*\n\s*Peak:\s*(-?[\d.]+) dBFS/.exec(audio.log);
    peak = loudest ? Number(loudest[1]) : null;
    loudness = integrated && Number.isFinite(Number(integrated[1])) ? Number(integrated[1]) : null;
  }

  let shots: Shots | null = null;
  let sheet: ContactSheet | null = null;
  if (info.width && info.height) {
    // Made small first: finding cuts doesn't need the full picture, and it is many times faster.
    const cuts = await run(
      ffmpeg,
      ["-hide_banner", "-nostats", "-i", file, "-an", "-vf", "scale=320:-2,select='gt(scene,0.3)',showinfo", "-f", "null", "-"],
      signal,
    );
    shots = cuts.code === 0 ? parseShots(cuts.log, info.duration) : null;

    const layout = sheetLayout(info.width, info.height);
    const frames = layout.columns * layout.rows;
    const made = await run(
      ffmpeg,
      [
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-i",
        file,
        "-an",
        "-vf",
        `fps=${frames}/${info.duration},scale=${layout.cellWidth}:${layout.cellHeight},tile=${layout.columns}x${layout.rows}`,
        "-frames:v",
        "1",
        "-q:v",
        "4",
        sheetFile,
      ],
      signal,
    );
    if (made.code === 0 && fs.existsSync(sheetFile)) sheet = { file: sheetFile, frames, step_seconds: round(info.duration / frames) };
  }

  if (signal?.aborted) throw new Error("Stopped.");
  return {
    version: STUDY_VERSION,
    file,
    size: stat.size,
    modified: stat.mtimeMs,
    duration: info.duration,
    width: info.width,
    height: info.height,
    aspect: info.aspect,
    fps: info.fps,
    loudness_lufs: loudness,
    true_peak_db: peak,
    shots,
    sound,
    sheet,
  };
}
