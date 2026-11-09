import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import type { TranscriptionStatus } from "../../shared/protocol.ts";
import { createTranscriber, describeResults } from "./transcriber.ts";
import type { Transcript } from "./whisper.ts";

function setup() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-transcriber-"));
  const media = (name: string, contents = "audio") => {
    const file = path.join(root, name);
    fs.writeFileSync(file, contents);
    return file;
  };
  const runs: string[] = [];
  let prepared = 0;
  const transcriber = createTranscriber({
    dir: path.join(root, "transcripts"),
    prepare: async () => {
      prepared += 1;
      return { ffmpeg: "ffmpeg", whisperCli: "whisper-cli", model: "m", vadModel: "v", label: "test" };
    },
    transcribeFile: async (_tools, file, language): Promise<Transcript> => {
      runs.push(`${path.basename(file)}:${language}`);
      if (file.endsWith("broken.wav")) throw new Error("Whisper failed");
      const stat = fs.statSync(file);
      return {
        file,
        size: stat.size,
        modified: stat.mtimeMs,
        engine: "test",
        language: language === "auto" ? "nl" : language,
        duration: 2,
        segments: [{ start: 0.5, end: 1.25, text: "Hallo daar.", words: [] }],
      };
    },
  });
  const statuses: TranscriptionStatus[] = [];
  transcriber.onStatus((status) => statuses.push(status));
  return { media, runs, transcriber, statuses, prepared: () => prepared };
}

describe("transcriber", () => {
  it("keeps a transcript until its file or the asked language changes", async () => {
    const { media, runs, transcriber, prepared } = setup();
    const file = media("interview.mov");

    await transcriber.transcribe([file]);
    await transcriber.transcribe([file]);
    await transcriber.transcribe([file], "nl");
    expect(runs).toEqual(["interview.mov:auto"]);

    await transcriber.transcribe([file], "en");
    fs.writeFileSync(file, "a longer recording");
    await transcriber.transcribe([file], "en");
    expect(runs).toEqual(["interview.mov:auto", "interview.mov:en", "interview.mov:en"]);
    expect(prepared()).toBe(1);
  });

  it("transcribes files one at a time, reports each failure on its own file, then goes idle", async () => {
    const { media, runs, transcriber, statuses } = setup();
    const results = await transcriber.transcribe([media("a.wav"), media("broken.wav"), media("b.wav"), path.join("missing.wav")]);

    expect(runs).toEqual(["a.wav:auto", "broken.wav:auto", "b.wav:auto"]);
    expect(results.map((result) => ("error" in result ? result.error : result.transcript.language))).toEqual([
      "nl",
      "Whisper failed",
      "nl",
      "File not found.",
    ]);
    expect(statuses.filter((status) => status.state === "transcribing").map((status) => status.done)).toEqual([0, 1, 2]);
    expect(statuses.at(-1)).toEqual({ state: "idle" });

    const text = describeResults(results);
    expect(text).toContain("[0.50-1.25] Hallo daar.");
    expect(text).toContain("Couldn't transcribe: Whisper failed");
  });
});
