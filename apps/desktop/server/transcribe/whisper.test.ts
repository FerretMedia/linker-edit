import * as fs from "node:fs";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import {
  buildSegments,
  detectionWindows,
  extractArgs,
  parseMediaInfo,
  parseVadSegments,
  pickLanguage,
  type WhisperJson,
} from "./whisper.ts";

// A real whisper-cli run (large-v3-turbo, voice detection on) over a Dutch screen recording, cut to three segments.
const fixture = JSON.parse(fs.readFileSync(path.join(import.meta.dirname, "fixtures", "whisper-nl.json"), "utf8")) as {
  log: string;
  json: WhisperJson;
};

describe("buildSegments", () => {
  it("puts words at their time in the file, not in the speech-only audio Whisper heard", () => {
    const segments = buildSegments(fixture.json, parseVadSegments(fixture.log));

    expect(segments.map((segment) => [segment.start, segment.text])).toEqual([
      [1.43, "Aha, dat is dus mijn camera."],
      // Starts in the silence whisper.cpp inserts between stretches of speech: speech resumes at 6.79.
      [6.79, "Oké, anyway, maar check even Tito."],
      [9.77, "Hell ja."],
    ]);
    expect(segments[0]!.words.map((word) => word.text)).toEqual(["Aha,", "dat", "is", "dus", "mijn", "camera."]);
    expect(segments[1]!.words.map((word) => word.text)).toEqual(["Oké,", "anyway,", "maar", "check", "even", "Tito."]);
    expect(segments[1]!.words[3]).toEqual({ start: 7.58, end: 7.85, text: "check" });
    for (const segment of segments) {
      for (const word of segment.words) expect(word.start).toBeLessThanOrEqual(word.end);
    }
  });
});

describe("language detection", () => {
  it("samples the whole clip and picks the language that wins overall, not the one it opens with", () => {
    expect(detectionWindows(40)).toEqual([]);
    expect(detectionWindows(200)).toEqual([0, 42.5, 85, 127.5, 170]);
    // whisper-cli -dl over those windows of a recording that opens in English and goes on in Dutch.
    const log = [
      "whisper_full_with_state: auto-detected language: en (p = 0.624113)",
      "whisper_full_with_state: auto-detected language: nl (p = 0.551040)",
      "whisper_full_with_state: auto-detected language: nl (p = 0.581648)",
      "whisper_full_with_state: auto-detected language: en (p = 0.575867)",
      "whisper_full_with_state: auto-detected language: nl (p = 0.927525)",
    ].join("\n");
    expect(pickLanguage(log)).toBe("nl");
    expect(pickLanguage("no detections")).toBeNull();
  });
});

describe("audio extraction", () => {
  it("mixes every audio stream into one, since recorders often put each mic on its own", () => {
    const log = [
      "  Duration: 00:01:02.60, start: 0.000000, bitrate: 6210 kb/s",
      "  Stream #0:0[0x1](und): Video: h264 (High), yuv420p, 1920x1080, 25 fps",
      "  Stream #0:1[0x2](und): Audio: pcm_s24le, 48000 Hz, mono, s32 (24 bit)",
      "  Stream #0:2[0x3](und): Audio: pcm_s24le, 48000 Hz, mono, s32 (24 bit)",
    ].join("\n");
    expect(parseMediaInfo(log)).toEqual({ audioStreams: 2, duration: 62.6 });
    expect(extractArgs("in.mxf", 2, "out.wav")).toContain("[0:a:0][0:a:1]amix=inputs=2");
    expect(extractArgs("in.mov", 1, "out.wav")).toContain("0:a:0");
  });
});
