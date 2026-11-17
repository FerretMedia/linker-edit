import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { createInterface } from "node:readline";
import type { WhisperTools } from "./tools.ts";

/** Times are seconds from the start of the source file. */
export interface Word {
  start: number;
  end: number;
  text: string;
}

export interface Segment {
  start: number;
  end: number;
  text: string;
  words: Word[];
}

export interface Transcript {
  file: string;
  /** Size and modification time of the file when it was transcribed, to spot a changed file. */
  size: number;
  modified: number;
  engine: string;
  language: string;
  duration: number;
  segments: Segment[];
}

/** A stretch of speech that voice detection kept, in the original audio and in the audio Whisper heard. */
export interface VadSegment {
  origStart: number;
  origEnd: number;
  vadStart: number;
  vadEnd: number;
}

interface WhisperToken {
  text: string;
  offsets: { from: number; to: number };
}

/** The parts of whisper-cli's --output-json-full that we read. */
export interface WhisperJson {
  result: { language: string };
  transcription: { offsets: { from: number; to: number }; text: string; tokens: WhisperToken[] }[];
}

/** whisper-cli logs each kept stretch of speech; we need them because it leaves word times unmapped. */
export function parseVadSegments(log: string): VadSegment[] {
  const pattern = /vad_segment_info: orig_start: ([\d.]+), orig_end: ([\d.]+), vad_start: ([\d.]+), vad_end: ([\d.]+)/g;
  return [...log.matchAll(pattern)].map((match) => ({
    origStart: Number(match[1]),
    origEnd: Number(match[2]),
    vadStart: Number(match[3]),
    vadEnd: Number(match[4]),
  }));
}

// whisper.cpp keeps 0.1 s of real audio after each stretch of speech, then inserts 0.1 s of silence.
const VAD_OVERLAP = 0.1;

/**
 * Maps a time in the speech-only audio Whisper heard back to the original file. In the inserted silence,
 * a word's start belongs to the next stretch of speech and its end to the previous one. whisper.cpp's own
 * mapping spreads those times across the removed silence instead, up to seconds off.
 */
export function toOriginalTime(seconds: number, vad: VadSegment[], edge: "start" | "end" = "start"): number {
  let previous: VadSegment | undefined;
  for (const segment of vad) {
    if (seconds < segment.vadStart) {
      return edge === "end" && previous ? previous.origEnd + VAD_OVERLAP : segment.origStart;
    }
    if (seconds <= segment.vadEnd + VAD_OVERLAP) return segment.origStart + (seconds - segment.vadStart);
    previous = segment;
  }
  return previous ? previous.origEnd + (seconds - previous.vadEnd) : seconds;
}

const round = (seconds: number) => Math.round(seconds * 1000) / 1000;

/**
 * Whisper's tokens are pieces of words: a new word starts at a token with a leading space. With voice
 * detection on, whisper.cpp leaves token times in the speech-only audio, so they are mapped here, and
 * segments span their words.
 */
export function buildSegments(json: WhisperJson, vad: VadSegment[]): Segment[] {
  const segments: Segment[] = [];
  for (const item of json.transcription) {
    const text = item.text.trim();
    if (!text) continue;
    const words: Word[] = [];
    for (const token of item.tokens) {
      if (!token.text || /^\[_.*\]$/.test(token.text)) continue;
      const from = toOriginalTime(token.offsets.from / 1000, vad);
      const to = Math.max(from, toOriginalTime(token.offsets.to / 1000, vad, "end"));
      const current = words.at(-1);
      if (!current || token.text.startsWith(" ")) {
        words.push({ start: from, end: to, text: token.text.trim() });
      } else {
        current.text += token.text;
        current.end = to;
      }
    }
    const kept = words.filter((word) => word.text).map((word) => ({ ...word, start: round(word.start), end: round(word.end) }));
    segments.push({
      start: kept[0]?.start ?? round(item.offsets.from / 1000),
      end: kept.at(-1)?.end ?? round(item.offsets.to / 1000),
      text,
      words: kept,
    });
  }
  return segments;
}

function run(file: string, args: string[]): Promise<{ code: number | null; log: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(file, args, { windowsHide: true });
    const lines: string[] = [];
    const collect = (line: string) => lines.push(line);
    createInterface({ input: child.stdout }).on("line", collect);
    createInterface({ input: child.stderr }).on("line", collect);
    child.on("error", reject);
    child.on("close", (code) => resolve({ code, log: lines.join("\n") }));
  });
}

const tail = (log: string) => log.split("\n").filter(Boolean).slice(-6).join("\n");

/** Audio streams and length, from ffmpeg's description of the file. */
export function parseMediaInfo(log: string): { audioStreams: number; duration: number } {
  const duration = /Duration: (\d+):(\d+):([\d.]+)/.exec(log);
  return {
    audioStreams: [...log.matchAll(/^\s*Stream #\d+:\d+.*: Audio:/gm)].length,
    duration: duration ? Number(duration[1]) * 3600 + Number(duration[2]) * 60 + Number(duration[3]) : 0,
  };
}

/** Cameras and recorders often put each mic on its own stream, so every audio stream is mixed in. */
export function extractArgs(file: string, audioStreams: number, wav: string): string[] {
  const mix =
    audioStreams > 1
      ? ["-filter_complex", `${Array.from({ length: audioStreams }, (_, i) => `[0:a:${i}]`).join("")}amix=inputs=${audioStreams}`]
      : ["-map", "0:a:0"];
  return ["-nostdin", "-hide_banner", "-v", "error", "-y", "-i", file, ...mix, "-vn", "-ac", "1", "-ar", "16000", "-c:a", "pcm_s16le", wav];
}

const WINDOW = 30;

/** Where to sample a clip for its language: Whisper on its own only listens to the first 30 seconds. */
export function detectionWindows(duration: number, count = 5): number[] {
  if (duration <= WINDOW * 1.5) return [];
  const windows = Math.min(count, Math.ceil(duration / WINDOW));
  return Array.from({ length: windows }, (_, i) => round(((duration - WINDOW) * i) / (windows - 1)));
}

/**
 * The language with the most confidence summed over the sampled windows. A clip that opens in English and
 * goes on in Dutch would otherwise be transcribed as English, with the Dutch translated, and badly.
 */
export function pickLanguage(log: string): string | null {
  const scores = new Map<string, number>();
  for (const match of log.matchAll(/auto-detected language: (\w+) \(p = ([\d.]+)\)/g)) {
    scores.set(match[1]!, (scores.get(match[1]!) ?? 0) + Number(match[2]));
  }
  return [...scores].sort((a, b) => b[1] - a[1])[0]?.[0] ?? null;
}

async function detectLanguage(tools: WhisperTools, wav: string, duration: number, work: string): Promise<string> {
  const windows = detectionWindows(duration);
  if (windows.length === 0) return "auto";
  const snippets: string[] = [];
  for (const [index, start] of windows.entries()) {
    const snippet = path.join(work, `window-${index}.wav`);
    const cut = await run(tools.ffmpeg, ["-nostdin", "-v", "error", "-y", "-ss", String(start), "-t", String(WINDOW), "-i", wav, snippet]);
    if (cut.code === 0) snippets.push(snippet);
  }
  // One whisper-cli run for all windows, so the model loads once.
  const detect = await run(tools.whisperCli, ["-m", tools.model, "-l", "auto", "-dl", "--vad", "-vm", tools.vadModel, ...snippets]);
  return pickLanguage(detect.log) ?? "auto";
}

/** Transcribes one media file on this machine. `language` is a Whisper code such as "nl", or "auto". */
export async function transcribeFile(
  tools: WhisperTools,
  file: string,
  language: string,
): Promise<Transcript> {
  const stat = fs.statSync(file);
  const info = parseMediaInfo((await run(tools.ffmpeg, ["-hide_banner", "-i", file])).log);
  if (info.audioStreams === 0) throw new Error("This file has no audio.");

  const work = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-transcribe-"));
  try {
    const wav = path.join(work, "audio.wav");
    const extract = await run(tools.ffmpeg, extractArgs(file, info.audioStreams, wav));
    if (extract.code !== 0) throw new Error(`ffmpeg couldn't read the audio:\n${tail(extract.log)}`);
    if (language === "auto") language = await detectLanguage(tools, wav, info.duration, work);

    const out = path.join(work, "transcript");
    const threads = String(Math.min(8, os.availableParallelism()));
    const whisper = await run(
      tools.whisperCli,
      ["-m", tools.model, "-f", wav, "-l", language, "--vad", "-vm", tools.vadModel, "-ojf", "-of", out, "-t", threads],
    );
    const jsonFile = `${out}.json`;
    if (whisper.code !== 0 || !fs.existsSync(jsonFile)) {
      // With no speech at all, voice detection leaves Whisper nothing to transcribe.
      if (/no speech segments|detected 0 speech segments/i.test(whisper.log)) {
        return { file, size: stat.size, modified: stat.mtimeMs, engine: tools.label, language, duration: info.duration, segments: [] };
      }
      throw new Error(`Whisper failed:\n${tail(whisper.log)}`);
    }
    const json = JSON.parse(fs.readFileSync(jsonFile, "utf8")) as WhisperJson;
    return {
      file,
      size: stat.size,
      modified: stat.mtimeMs,
      engine: tools.label,
      language: json.result.language,
      duration: info.duration,
      segments: buildSegments(json, parseVadSegments(whisper.log)),
    };
  } finally {
    fs.rmSync(work, { recursive: true, force: true });
  }
}
