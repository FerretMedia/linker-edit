import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import ffmpegPath from "ffmpeg-static";

export interface WhisperTools {
  ffmpeg: string;
  whisperCli: string;
  model: string;
  vadModel: string;
  /** Names the engine in each transcript. */
  label: string;
}

/** What setup is doing, for the status line: a download with its percent, or null while unpacking. */
export type SetupProgress = (step: string, percent: number | null) => void;

interface Download {
  url: string;
  sha256: string;
  size: number;
}

// Pinned builds and models, checked by hash, so every machine runs the same tested engine.
const WHISPER_BUILD = "b5454";
const WHISPER_RELEASE = `https://github.com/ggml-org/whisper.cpp/releases/download/${WHISPER_BUILD}`;
const WINDOWS_BUILDS = {
  cuda: {
    url: `${WHISPER_RELEASE}/whisper-bin-win-cuda-12.4.0-x64.zip`,
    sha256: "afef0b881c500958921c3f5523b50e59ee2ec9b6f5cbd25b324c51ed308a957a",
    size: 684_913_404,
  },
  cpu: {
    url: `${WHISPER_RELEASE}/whisper-bin-x64.zip`,
    sha256: "6ba69e3482d7826214f90a6a9c84ca07782aec1e1d0c6a7c30c994fd5d816ccb",
    size: 8_928_640,
  },
} satisfies Record<string, Download>;

const MODEL_NAME = "large-v3-turbo";
const MODEL: Download = {
  url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/5359861c739e955e79d9a303bcbc70fb988958b1/ggml-large-v3-turbo-q8_0.bin",
  sha256: "317eb69c11673c9de1e1f0d459b253999804ec71ac4c23c17ecf5fbe24e259a1",
  size: 874_188_075,
};
const VAD_MODEL: Download = {
  url: "https://huggingface.co/ggml-org/whisper-vad/resolve/9ffd54a1e1ee413ddf265af9913beaf518d1639b/ggml-silero-v5.1.2.bin",
  sha256: "29940d98d42b91fbd05ce489f3ecf7c72f0a42f027e4875919a28fb4c04ea2cf",
  size: 885_098,
};

/** Downloads to a .part file and only keeps it when the hash matches, so a broken download is retried. */
async function download(item: Download, target: string, onPercent: (percent: number) => void): Promise<void> {
  if (fs.existsSync(target)) return;
  fs.mkdirSync(path.dirname(target), { recursive: true });
  const response = await fetch(item.url);
  if (!response.ok || !response.body) throw new Error(`Download failed (${response.status}): ${item.url}`);
  const part = `${target}.part`;
  const hash = createHash("sha256");
  const out = fs.createWriteStream(part);
  let received = 0;
  let reported = -1;
  try {
    for await (const chunk of response.body) {
      hash.update(chunk);
      received += chunk.length;
      if (!out.write(chunk)) await new Promise((resolve) => out.once("drain", resolve));
      const percent = Math.floor((received / item.size) * 100);
      if (percent !== reported) onPercent((reported = percent));
    }
    await new Promise<void>((resolve, reject) => out.end((error?: Error | null) => (error ? reject(error) : resolve())));
  } catch (error) {
    out.destroy();
    fs.rmSync(part, { force: true });
    throw error;
  }
  if (hash.digest("hex") !== item.sha256) {
    fs.rmSync(part, { force: true });
    throw new Error(`The download of ${path.basename(target)} was damaged. Try again.`);
  }
  fs.renameSync(part, target);
}

function succeeds(file: string, args: string[]): Promise<boolean> {
  return new Promise((resolve) => execFile(file, args, { windowsHide: true, timeout: 10_000 }, (error) => resolve(!error)));
}

function findOnPath(name: string): string | null {
  const names = process.platform === "win32" ? [`${name}.exe`, name] : [name];
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    for (const candidate of names) {
      const file = path.join(dir, candidate);
      if (dir && fs.existsSync(file)) return file;
    }
  }
  return null;
}

async function bundledWindowsWhisper(dir: string, onProgress: SetupProgress): Promise<{ cli: string; gpu: boolean }> {
  const gpu = await succeeds("nvidia-smi", ["-L"]);
  const kind = gpu ? "cuda" : "cpu";
  const home = path.join(dir, `whisper.cpp-${WHISPER_BUILD}-${kind}`);
  const cli = path.join(home, "Release", "whisper-cli.exe");
  if (fs.existsSync(cli)) return { cli, gpu };

  const zip = `${home}.zip`;
  await download(WINDOWS_BUILDS[kind], zip, (percent) => onProgress("Downloading the speech engine", percent));
  onProgress("Unpacking the speech engine", null);
  const unpacking = `${home}.unpacking`;
  fs.rmSync(unpacking, { recursive: true, force: true });
  fs.mkdirSync(unpacking, { recursive: true });
  // Windows' own tar reads zip files; a Git Bash tar earlier on PATH does not.
  const tar = path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe");
  if (!(await succeeds(tar, ["-xf", zip, "-C", unpacking]))) throw new Error(`Couldn't unpack ${zip}.`);
  fs.renameSync(unpacking, home);
  fs.rmSync(zip, { force: true });
  return { cli, gpu };
}

const MISSING_WHISPER =
  process.platform === "darwin"
    ? "Install whisper.cpp with `brew install whisper-cpp`, or set whisperCli in linker-edit.config.json."
    : "Install whisper.cpp so `whisper-cli` is on PATH, or set whisperCli in linker-edit.config.json.";

/** The ffmpeg Linker-Edit ships, or one on PATH. */
export function findFfmpeg(): string {
  // ffmpeg-static downloads its binary in an install script, which npm can be set to skip.
  const ffmpeg = ffmpegPath && fs.existsSync(ffmpegPath) ? ffmpegPath : findOnPath("ffmpeg");
  if (!ffmpeg) throw new Error("ffmpeg is missing. Reinstall Linker-Edit, or install ffmpeg so it's on your PATH.");
  return ffmpeg;
}

/** Finds or downloads ffmpeg, whisper.cpp and its models. The first run downloads about 1.5 GB on Windows. */
export async function prepareTools(dir: string, override: string | undefined, onProgress: SetupProgress): Promise<WhisperTools> {
  const ffmpeg = findFfmpeg();
  let whisperCli = override ?? null;
  let device = "";
  if (!whisperCli && process.platform === "win32" && process.arch === "x64") {
    const bundled = await bundledWindowsWhisper(dir, onProgress);
    whisperCli = bundled.cli;
    device = bundled.gpu ? " (CUDA)" : " (CPU)";
  }
  whisperCli ??= findOnPath("whisper-cli");
  if (!whisperCli || !fs.existsSync(whisperCli)) throw new Error(`whisper.cpp not found. ${MISSING_WHISPER}`);

  const model = path.join(dir, "models", path.basename(MODEL.url));
  const vadModel = path.join(dir, "models", path.basename(VAD_MODEL.url));
  await download(MODEL, model, (percent) => onProgress("Downloading the speech model", percent));
  await download(VAD_MODEL, vadModel, (percent) => onProgress("Downloading the voice detector", percent));
  return { ffmpeg, whisperCli, model, vadModel, label: `whisper.cpp ${MODEL_NAME}${device}` };
}
