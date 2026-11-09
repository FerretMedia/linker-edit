import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import type { TranscriptionStatus } from "../../shared/protocol.ts";
import type { SetupProgress, WhisperTools } from "./tools.ts";
import { transcribeFile, type Transcript } from "./whisper.ts";

export interface TranscriberOptions {
  /** Where transcripts are kept, one .json (with word times) and one .txt per media file. */
  dir: string;
  /** Finds or downloads the engine; called once, again only after it failed. */
  prepare: (onProgress: SetupProgress) => Promise<WhisperTools>;
  transcribeFile?: typeof transcribeFile;
}

export type TranscriptResult =
  | { file: string; transcript: Transcript; textFile: string; jsonFile: string }
  | { file: string; error: string };

export type Transcriber = ReturnType<typeof createTranscriber>;

const normalize = (file: string) => (process.platform === "win32" ? path.resolve(file).toLowerCase() : path.resolve(file));

function seconds(value: number): string {
  const minutes = Math.floor(value / 60);
  return `${minutes}:${String(Math.round(value % 60)).padStart(2, "0")}`;
}

/** The transcript as agents read it: one line per segment, with times in seconds. */
export function transcriptText(transcript: Transcript): string {
  const lines = [
    transcript.file,
    `Language: ${transcript.language}. Length: ${seconds(transcript.duration)}. Engine: ${transcript.engine}.`,
    "Times are seconds from the start of the source file.",
    "",
    ...transcript.segments.map((segment) => `[${segment.start.toFixed(2)}-${segment.end.toFixed(2)}] ${segment.text}`),
  ];
  if (transcript.segments.length === 0) lines.push("(no speech found)");
  return `${lines.join("\n")}\n`;
}

/** Transcripts go inline while they fit in a tool result; longer ones are left in their files. */
export function describeResults(results: TranscriptResult[], budget = 40_000): string {
  const parts: string[] = [];
  let left = budget;
  for (const result of results) {
    if ("error" in result) {
      parts.push(`${result.file}\nCouldn't transcribe: ${result.error}`);
      continue;
    }
    const text = transcriptText(result.transcript);
    const files = `Transcript: ${result.textFile}\nWord timings: ${result.jsonFile}`;
    if (text.length <= left) {
      left -= text.length;
      parts.push(`${text}${files}`);
    } else {
      const { language, segments } = result.transcript;
      parts.push(`${result.file}\nLanguage: ${language}, ${segments.length} segments, too long to show here.\n${files}`);
    }
  }
  return parts.join("\n\n---\n\n");
}

/** Transcribes media files one at a time, since they share the GPU, and keeps each transcript until its file changes. */
export function createTranscriber(options: TranscriberOptions) {
  let status: TranscriptionStatus = { state: "idle" };
  const listeners = new Set<(status: TranscriptionStatus) => void>();
  let tools: Promise<WhisperTools> | null = null;
  let queue: Promise<unknown> = Promise.resolve();
  let done = 0;
  let total = 0;
  const pending = new Map<string, Promise<TranscriptResult>>();

  function setStatus(next: TranscriptionStatus) {
    status = next;
    for (const listener of listeners) listener(next);
  }

  function prepare(): Promise<WhisperTools> {
    tools ??= options.prepare((step, percent) => setStatus({ state: "preparing", step, percent })).catch((error: unknown) => {
      tools = null;
      throw error;
    });
    return tools;
  }

  function cacheFiles(file: string) {
    const key = createHash("sha256").update(normalize(file)).digest("hex").slice(0, 12);
    const name = path.parse(file).name.replace(/[^\w.-]+/g, "_").slice(0, 60);
    const base = path.join(options.dir, `${name}-${key}`);
    return { jsonFile: `${base}.json`, textFile: `${base}.txt` };
  }

  function cached(file: string, language: string): Transcript | null {
    try {
      const transcript = JSON.parse(fs.readFileSync(cacheFiles(file).jsonFile, "utf8")) as Transcript;
      const stat = fs.statSync(file);
      if (transcript.size !== stat.size || transcript.modified !== stat.mtimeMs) return null;
      if (language !== "auto" && transcript.language !== language) return null;
      return transcript;
    } catch {
      return null;
    }
  }

  function save(file: string, transcript: Transcript): TranscriptResult {
    const files = cacheFiles(file);
    fs.mkdirSync(options.dir, { recursive: true });
    fs.writeFileSync(files.jsonFile, JSON.stringify(transcript));
    fs.writeFileSync(files.textFile, transcriptText(transcript));
    return { file, transcript, ...files };
  }

  function transcribeOne(file: string, language: string): Promise<TranscriptResult> {
    const key = `${normalize(file)}\n${language}`;
    const running = pending.get(key);
    if (running) return running;
    if (!fs.existsSync(file) || !fs.statSync(file).isFile()) return Promise.resolve({ file, error: "File not found." });
    const hit = cached(file, language);
    if (hit) return Promise.resolve({ file, transcript: hit, ...cacheFiles(file) });

    total += 1;
    const job = queue.then(async (): Promise<TranscriptResult> => {
      try {
        const ready = await prepare();
        setStatus({ state: "transcribing", file: path.basename(file), done, total });
        const transcript = await (options.transcribeFile ?? transcribeFile)(ready, file, language);
        return save(file, transcript);
      } catch (error) {
        return { file, error: error instanceof Error ? error.message : String(error) };
      } finally {
        pending.delete(key);
        done += 1;
        if (done === total) {
          done = total = 0;
          setStatus({ state: "idle" });
        }
      }
    });
    queue = job;
    pending.set(key, job);
    return job;
  }

  return {
    status: () => status,
    onStatus(listener: (status: TranscriptionStatus) => void): () => void {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    /** `language` is a Whisper code such as "en" or "nl"; "auto" detects it per file. */
    transcribe(files: string[], language = "auto"): Promise<TranscriptResult[]> {
      return Promise.all(files.map((file) => transcribeOne(path.resolve(file), language.trim().toLowerCase() || "auto")));
    },
  };
}
