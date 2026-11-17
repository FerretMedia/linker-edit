import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import type { TranscriptResult } from "../transcribe/transcriber.ts";
import type { Transcript } from "../transcribe/whisper.ts";
import { compareCut } from "./compare.ts";
import { parseShots, parseSound, parseVideoInfo, sheetLayout, STUDY_VERSION, type Study } from "./measure.ts";
import { createStudy, videoFiles } from "./study.ts";

/** A transcript of evenly spoken words; "|" is a two-second pause. */
function transcript(file: string, text: string): Transcript {
  let at = 0;
  const words = text.split(/\s+/).flatMap((word) => {
    if (word === "|") {
      at += 2;
      return [];
    }
    const start = at;
    at += 0.5;
    return [{ start, end: start + 0.4, text: word }];
  });
  return { file, size: 1, modified: 1, engine: "test", language: "en", duration: at, segments: [{ start: 0, end: at, text, words }] };
}

describe("measuring a video", () => {
  it("reads the picture as it is shown, the frame rate and whether there is sound", () => {
    const log = `Input #0, mov,mp4,m4a,3gp,3g2,mj2, from 'reel.mp4':
  Duration: 00:00:34.50, start: 0.000000, bitrate: 8000 kb/s
  Stream #0:0[0x1](und): Video: h264 (High) (avc1 / 0x31637661), yuv420p(tv, bt709, progressive), 1920x1080 [SAR 1:1 DAR 16:9], 7800 kb/s, 29.97 fps, 29.97 tbr, 30k tbn (default)
    Side data:
      displaymatrix: rotation of -90.00 degrees
  Stream #0:1[0x2](und): Audio: aac (LC) (mp4a / 0x6134706D), 48000 Hz, stereo, fltp, 192 kb/s (default)`;
    // A phone records sideways and says so: the video is tall.
    expect(parseVideoInfo(log)).toEqual({ duration: 34.5, width: 1080, height: 1920, aspect: "9:16", fps: 29.97, audio: true });
    expect(parseVideoInfo(log.replace(/\n {4}Side data:\n.*degrees/, "").replace(/\n.*Audio:.*/, ""))).toMatchObject({
      width: 1920,
      height: 1080,
      aspect: "16:9",
      audio: false,
    });
    expect(parseVideoInfo("reel.mp4: Invalid data found when processing input").duration).toBe(0);
  });

  it("turns the times the picture changes into shot lengths", () => {
    const log = "[Parsed_showinfo_2] n:0 pts:60 pts_time:2 \n[Parsed_showinfo_2] n:1 pts:150 pts_time:5\n[Parsed_showinfo_2] n:2 pts:300 pts_time:10";
    expect(parseShots(log, 20)).toEqual({ count: 4, average_seconds: 5, median_seconds: 5, longest_seconds: 10, cuts_per_minute: 9, cuts: [2, 5, 10] });
    // One shot, start to end.
    expect(parseShots("", 12)).toMatchObject({ count: 1, average_seconds: 12, cuts: [] });
    expect(parseShots("", 0)).toBeNull();
  });

  it("says how quiet a video gets", () => {
    const log = `  Duration: 00:00:20.00, start: 0.000000
[silencedetect @ 1] silence_start: 4
[silencedetect @ 1] silence_end: 5 | silence_duration: 1
[silencedetect @ 1] silence_start: 10
[silencedetect @ 1] silence_end: 13 | silence_duration: 3`;
    expect(parseSound(log)).toEqual({ quiet_share: 0.2, pauses: 2, longest_pause_seconds: 3, median_pause_seconds: 3 });
    // Music under everything: never quiet.
    expect(parseSound("  Duration: 00:00:20.00")).toEqual({ quiet_share: 0, pauses: 0, longest_pause_seconds: 0, median_pause_seconds: 0 });
  });

  it("lays a tall video's contact sheet out in more, narrower frames", () => {
    expect(sheetLayout(1920, 1080)).toEqual({ columns: 6, rows: 4, cellWidth: 320, cellHeight: 180 });
    expect(sheetLayout(1080, 1920)).toEqual({ columns: 8, rows: 3, cellWidth: 180, cellHeight: 320 });
  });
});

describe("raw footage against the finished video", () => {
  const raw = transcript(
    "/shoot/a.mov",
    "Hello, and welcome to the show today we talk about cameras | um let me start again | hello and welcome to the show today we talk about lenses and tripods | this part is boring nobody cares about my cat at all really",
  );

  it("finds what was kept, which take it was, and what was cut and why", () => {
    const result = compareCut(transcript("/final.mp4", "Hello and welcome to the show. Today we talk about lenses and tripods!"), [raw]);
    expect(result.kept).toEqual([
      {
        final: [0, 6.4],
        raw_file: "/shoot/a.mov",
        raw: [12, 18.4],
        words: 13,
        text: "hello and welcome to the show today we talk about lenses and tripods",
        // Said twice in the raw footage; the editor used the second take.
        take: { used: 2, of: 2 },
      },
    ]);
    expect(result).toMatchObject({ raw_words: 41, kept_share: 0.32, not_from_raw_share: 0, parts: 1, reordered: 0 });
    expect(result.cut.map((part) => [part.kind, part.words, part.raw])).toEqual([
      ["retake", 16, [0, 9.9]],
      ["content", 12, [20.5, 26.4]],
    ]);
  });

  it("counts jumps back in the raw footage as reordering, and words from elsewhere as not from it", () => {
    const final = transcript("/final.mp4", "nobody cares about my cat | and now a word from our sponsor | hello and welcome to the show today we talk about lenses");
    const result = compareCut(final, [raw]);
    expect(result.kept.map((part) => part.text)).toEqual(["nobody cares about my cat", "hello and welcome to the show today we talk about lenses"]);
    expect(result).toMatchObject({ parts: 2, reordered: 1, not_from_raw_share: 0.3 });
    // The silence left where the two parts meet, sponsor read included.
    expect(result.median_gap_at_cuts_seconds).toBeGreaterThan(2);
  });

  it("keeps apart raw files, and joins a part split by one misheard word", () => {
    const b = transcript("/shoot/b.mov", "the second camera angle shows the product from the side");
    const final = transcript("/final.mp4", "hello and welcome to the snow today we talk about lenses the second camera angle shows the product");
    const result = compareCut(final, [raw, b]);
    expect(result.kept.map((part) => [path.basename(part.raw_file), part.words])).toEqual([
      ["a.mov", 11],
      ["b.mov", 7],
    ]);
    expect(result.reordered).toBe(1);
  });
});

describe("the study tools", () => {
  function setup() {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-study-"));
    const videos = path.join(root, "videos");
    fs.mkdirSync(path.join(videos, "inside"), { recursive: true });
    for (const name of ["b.mp4", "a.MOV", "notes.txt", ".hidden.mp4", "inside/c.mp4"]) fs.writeFileSync(path.join(videos, name), name);
    const measured: string[] = [];
    const study = createStudy({
      dir: path.join(root, "cache"),
      ffmpeg: () => "ffmpeg",
      transcribe: async (files): Promise<TranscriptResult[]> =>
        files.map((file) =>
          file.endsWith("b.mp4")
            ? { file, error: "No audio." }
            : { file, transcript: transcript(file, "so today we are testing three lenses"), textFile: `${file}.txt`, jsonFile: `${file}.json` },
        ),
      measure: async (_ffmpeg, file, sheetFile): Promise<Study> => {
        measured.push(path.basename(file));
        if (file.endsWith("broken.mp4")) throw new Error("ffmpeg couldn't read it.");
        fs.writeFileSync(sheetFile, "jpeg");
        const stat = fs.statSync(file);
        return {
          version: STUDY_VERSION,
          file,
          size: stat.size,
          modified: stat.mtimeMs,
          duration: 30,
          width: 1080,
          height: 1920,
          aspect: "9:16",
          fps: 30,
          loudness_lufs: -14,
          shots: null,
          sound: null,
          sheet: { file: sheetFile, frames: 24, step_seconds: 1.25 },
        };
      },
    });
    return { root, videos, study, measured };
  }

  it("lists a folder's videos in name order, one level deep", () => {
    const { videos } = setup();
    expect(videoFiles([videos]).map((file) => path.basename(file))).toEqual(["a.MOV", "b.mp4"]);
    expect(() => videoFiles([path.join(videos, "gone")])).toThrow(/no file or folder/);
    expect(() => videoFiles([path.join(videos, "inside", "..", "inside", "..", "..", "cache")])).toThrow(/no file or folder/);
  });

  it("measures each video once, shows its contact sheet, and says what a failed one failed on", async () => {
    const { videos, study, measured } = setup();
    fs.writeFileSync(path.join(videos, "broken.mp4"), "x");
    const first = await study.run("study_media", { files: [videos] });
    const result = JSON.parse(first.text) as { videos: Record<string, unknown>[] };
    expect(result.videos.map((video) => path.basename(String(video.file)))).toEqual(["a.MOV", "b.mp4", "broken.mp4"]);
    expect(result.videos[0]).toMatchObject({
      aspect: "9:16",
      duration: 30,
      contact_sheet: { image: 1, frames: 24, step_seconds: 1.25 },
      speech: { words: 7, words_per_minute: 120, first_word_at_seconds: 0, opens_with: "so today we are testing three lenses" },
    });
    // What a cached measurement is checked against stays out of the agent's context.
    expect(result.videos[0]).not.toHaveProperty("modified");
    expect(result.videos[1]).toMatchObject({ speech: null, transcript_problem: "No audio.", contact_sheet: { image: 2 } });
    expect(result.videos[2]).toEqual({ file: path.join(videos, "broken.mp4"), error: "ffmpeg couldn't read it." });
    expect(first.images).toHaveLength(2);

    // Asked again, only the one that failed is measured again; a changed file is too.
    await study.run("study_media", { files: [videos], transcribe: false });
    expect(measured).toEqual(["a.MOV", "b.mp4", "broken.mp4", "broken.mp4"]);
    fs.writeFileSync(path.join(videos, "a.MOV"), "a longer file now");
    await study.run("study_media", { files: [path.join(videos, "a.MOV")], transcribe: false });
    expect(measured.at(-1)).toBe("a.MOV");
  });

  it("studies eight videos a call and names the rest", async () => {
    const { root, study } = setup();
    const many = path.join(root, "many");
    fs.mkdirSync(many);
    for (let i = 0; i < 10; i += 1) fs.writeFileSync(path.join(many, `v${i}.mp4`), "x");
    const result = JSON.parse((await study.run("study_media", { files: [many], transcribe: false })).text) as { videos: unknown[]; not_studied_yet: string[] };
    expect(result.videos).toHaveLength(8);
    expect(result.not_studied_yet.map((file) => path.basename(file))).toEqual(["v8.mp4", "v9.mp4"]);
  });

  it("compares a finished video with its raw footage, and says what it needs when it can't", async () => {
    const { videos, study } = setup();
    const compared = JSON.parse((await study.run("compare_cut", { final: path.join(videos, "a.MOV"), raw: [path.join(videos, "inside")] })).text) as { kept_share: number };
    expect(compared.kept_share).toBe(1);
    await expect(study.run("compare_cut", { final: path.join(videos, "b.mp4"), raw: [path.join(videos, "inside")] })).rejects.toThrow(/Couldn't transcribe/);
    await expect(study.run("compare_cut", { final: path.join(videos, "a.MOV"), raw: [] })).rejects.toThrow(/raw footage/);
  });
});
