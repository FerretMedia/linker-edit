// Work on media files that needs no Resolve: silences and frames, with the ffmpeg Linker-Edit ships.
import { execFile } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";

function run(ffmpeg: string, args: string[], timeout = 10 * 60_000): Promise<{ code: number; stderr: string }> {
  return new Promise((resolve) => {
    execFile(ffmpeg, args, { windowsHide: true, timeout, maxBuffer: 64 * 1024 * 1024 }, (error, _stdout, stderr) =>
      resolve({ code: error ? ((error as { code?: number }).code ?? 1) : 0, stderr: String(stderr) }),
    );
  });
}

const round = (value: number) => Math.round(value * 1000) / 1000;

export interface Silences {
  file: string;
  /** Seconds. */
  duration: number;
  /** [start, end] in seconds from the file's start, already shrunk by the padding. */
  silences: [number, number][];
  /** What's between the silences. */
  speech: [number, number][];
  silent_seconds: number;
}

/** Reads ffmpeg's silencedetect log. A silence still open at the end runs to the end of the file. */
export function parseSilences(log: string, padding: number): Omit<Silences, "file"> {
  const time = /Duration: (\d+):(\d+):([\d.]+)/.exec(log);
  const duration = time ? Number(time[1]) * 3600 + Number(time[2]) * 60 + Number(time[3]) : 0;
  const raw: [number, number][] = [];
  let open: number | null = null;
  for (const match of log.matchAll(/silence_(start|end): (-?[\d.]+)/g)) {
    const value = Math.max(0, Number(match[2]));
    if (match[1] === "start") open = value;
    else if (open !== null) {
      raw.push([open, value]);
      open = null;
    }
  }
  if (open !== null && duration > open) raw.push([open, duration]);
  // Keep a little of each pause next to speech, except at the very start and end of the file.
  const silences = raw
    .map(([start, end]): [number, number] => [start <= 0 ? 0 : start + padding, end >= duration ? duration : end - padding])
    .filter(([start, end]) => end - start > 0.01)
    .map(([start, end]): [number, number] => [round(start), round(end)]);
  const speech: [number, number][] = [];
  let at = 0;
  for (const [start, end] of silences) {
    if (start > at) speech.push([round(at), round(start)]);
    at = end;
  }
  if (duration > at) speech.push([round(at), round(duration)]);
  const silent = silences.reduce((sum, [start, end]) => sum + end - start, 0);
  return { duration: round(duration), silences, speech, silent_seconds: round(silent) };
}

export async function findSilences(
  ffmpeg: string,
  file: string,
  { minSilence = 0.5, thresholdDb = -35, padding = 0.1 }: { minSilence?: number; thresholdDb?: number; padding?: number } = {},
): Promise<Silences> {
  const { code, stderr } = await run(ffmpeg, ["-hide_banner", "-nostats", "-i", file, "-vn", "-af", `silencedetect=noise=${thresholdDb}dB:d=${minSilence}`, "-f", "null", "-"]);
  if (code !== 0 && !/silence|Duration/.test(stderr)) {
    throw new Error(`ffmpeg couldn't read ${file}: ${stderr.trim().split("\n").at(-1) ?? ""}`);
  }
  if (!/Audio:/.test(stderr)) throw new Error(`${file} has no audio.`);
  return { file, ...parseSilences(stderr, padding) };
}

/** A frame of a media file as a JPEG at most 640 pixels wide. */
export async function mediaFrame(ffmpeg: string, file: string, seconds: number, out: string): Promise<boolean> {
  const { code } = await run(ffmpeg, ["-hide_banner", "-loglevel", "error", "-y", "-ss", String(Math.max(0, seconds)), "-i", file, "-frames:v", "1", "-vf", "scale='min(640,iw)':-1", "-q:v", "4", out], 60_000);
  return code === 0 && fs.existsSync(out);
}

/** A still Resolve exported, made small for the agent: a JPEG at most 640 pixels wide. Keeps the original when ffmpeg can't. */
export async function shrinkImage(ffmpeg: string | null, file: string): Promise<string> {
  if (!ffmpeg) return file;
  const out = file.replace(/\.[^.]+$/, ".jpg");
  const { code } = await run(ffmpeg, ["-hide_banner", "-loglevel", "error", "-y", "-i", file, "-vf", "scale='min(640,iw)':-1", "-q:v", "4", out], 60_000);
  if (code !== 0 || !fs.existsSync(out)) return file;
  fs.rmSync(file, { force: true });
  return out;
}

/** A fresh folder for frames, under one that drops folders older than an hour. */
export function framesDir(root: string): string {
  fs.mkdirSync(root, { recursive: true });
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const full = path.join(root, entry.name);
    if (entry.isDirectory() && Date.now() - fs.statSync(full).mtimeMs > 60 * 60_000) fs.rmSync(full, { recursive: true, force: true });
  }
  return fs.mkdtempSync(path.join(root, "frames-"));
}
