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

export interface Download {
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
/** A whisper-cli packed in a .tar.gz, for Linux and macOS. */
export interface TarBuild extends Download {
  /** Where whisper-cli is inside the archive. */
  cli: string;
  /** How it runs, for the engine's label. */
  device: string;
}

// whisper.cpp publishes no macOS whisper-cli, so .github/workflows/whisper-macos.yml builds these from the same
// whisper.cpp build: Metal on Apple silicon, the CPU on Intel Macs.
const MAC_RELEASE = `https://github.com/FerretMedia/linker-edit/releases/download/whisper.cpp-${WHISPER_BUILD}-macos`;

// A whisper-cli on PATH comes first: on Linux that may be a GPU build, on macOS Homebrew's.
const TAR_BUILDS: Partial<Record<NodeJS.Platform, Partial<Record<NodeJS.Architecture, TarBuild>>>> = {
  // CPU builds, which run on any recent glibc.
  linux: {
    x64: {
      url: `${WHISPER_RELEASE}/whisper-bin-ubuntu-x64.tar.gz`,
      sha256: "a72becf15d7917f990f6313867a52638b82b7f9ef237fb0c980dac56a135781c",
      size: 10_364_195,
      cli: "whisper-bin-ubuntu-x64/whisper-cli",
      device: "CPU",
    },
    arm64: {
      url: `${WHISPER_RELEASE}/whisper-bin-ubuntu-arm64.tar.gz`,
      sha256: "6b95ebfc60447df48e70ef00a73bdc3f41679ed2d01ef465827206a2ff325149",
      size: 4_608_377,
      cli: "whisper-bin-ubuntu-arm64/whisper-cli",
      device: "CPU",
    },
  },
  darwin: {
    arm64: {
      url: `${MAC_RELEASE}/whisper-cli-macos-arm64.tar.gz`,
      sha256: "e0788310660591d35410e50bd04f49db92348871db3af4bdca489b6edd57b735",
      size: 2_092_170,
      cli: "whisper-cli-macos-arm64/whisper-cli",
      device: "Metal",
    },
    x64: {
      url: `${MAC_RELEASE}/whisper-cli-macos-x64.tar.gz`,
      sha256: "c6309883922c831cb9c01c4697a615a43fec472f349b9d249c683847b6fc6d88",
      size: 1_337_492,
      cli: "whisper-cli-macos-x64/whisper-cli",
      device: "CPU",
    },
  },
};

/** The whisper.cpp download for this system when it comes as a .tar.gz, or null when there's none. */
export function tarBuild(platform: NodeJS.Platform, arch: NodeJS.Architecture): TarBuild | null {
  return TAR_BUILDS[platform]?.[arch] ?? null;
}

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
export async function download(item: Download, target: string, onPercent: (percent: number) => void, signal?: AbortSignal): Promise<void> {
  if (fs.existsSync(target)) return;
  fs.mkdirSync(path.dirname(target), { recursive: true });
  const response = await fetch(item.url, { signal });
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

function succeeds(file: string, args: string[], timeout = 10_000): Promise<boolean> {
  return new Promise((resolve) => execFile(file, args, { windowsHide: true, timeout }, (error) => resolve(!error)));
}

// The CUDA engine's zip is about 685 MB: unpacking it takes well over the 10 seconds a quick check gets, more on a
// slow disk or with a virus scanner reading along.
const UNPACK_TIMEOUT_MS = 15 * 60_000;

export function findOnPath(name: string): string | null {
  const names = process.platform === "win32" ? [`${name}.exe`, name] : [name];
  for (const dir of (process.env.PATH ?? "").split(path.delimiter)) {
    for (const candidate of names) {
      const file = path.join(dir, candidate);
      if (dir && fs.existsSync(file)) return file;
    }
  }
  return null;
}

/** Unpacks into a temporary folder first, so a half-unpacked engine is never taken for a working one. */
async function unpack(archive: string, home: string, tar: string, args: string[]): Promise<void> {
  const unpacking = `${home}.unpacking`;
  fs.rmSync(unpacking, { recursive: true, force: true });
  fs.mkdirSync(unpacking, { recursive: true });
  if (!(await succeeds(tar, [...args, archive, "-C", unpacking], UNPACK_TIMEOUT_MS))) throw new Error(`Couldn't unpack ${archive}.`);
  fs.renameSync(unpacking, home);
  fs.rmSync(archive, { force: true });
}

async function bundledWindowsWhisper(dir: string, onProgress: SetupProgress, signal?: AbortSignal): Promise<{ cli: string; gpu: boolean }> {
  const gpu = await succeeds("nvidia-smi", ["-L"]);
  const kind = gpu ? "cuda" : "cpu";
  const home = path.join(dir, `whisper.cpp-${WHISPER_BUILD}-${kind}`);
  const cli = path.join(home, "Release", "whisper-cli.exe");
  if (fs.existsSync(cli)) return { cli, gpu };

  const zip = `${home}.zip`;
  await download(WINDOWS_BUILDS[kind], zip, (percent) => onProgress("Downloading the speech engine", percent), signal);
  onProgress("Unpacking the speech engine", null);
  // Windows' own tar reads zip files; a Git Bash tar earlier on PATH does not.
  await unpack(zip, home, path.join(process.env.SystemRoot ?? "C:\\Windows", "System32", "tar.exe"), ["-xf"]);
  return { cli, gpu };
}

const tarHome = (dir: string) => path.join(dir, `whisper.cpp-${WHISPER_BUILD}-${process.platform}-${process.arch}`);

/** Where this system's downloaded whisper-cli is, or null when there's no .tar.gz build for it. */
function tarWhisperCli(dir: string): string | null {
  const build = tarBuild(process.platform, process.arch);
  return build ? path.join(tarHome(dir), ...build.cli.split("/")) : null;
}

async function bundledTarWhisper(dir: string, onProgress: SetupProgress, signal?: AbortSignal): Promise<{ cli: string; device: string } | null> {
  const build = tarBuild(process.platform, process.arch);
  const cli = tarWhisperCli(dir);
  if (!build || !cli) return null;
  if (fs.existsSync(cli)) return { cli, device: build.device };

  const home = tarHome(dir);
  const archive = `${home}.tar.gz`;
  await download(build, archive, (percent) => onProgress("Downloading the speech engine", percent), signal);
  onProgress("Unpacking the speech engine", null);
  // macOS's own tar, wherever PATH points.
  await unpack(archive, home, process.platform === "darwin" ? "/usr/bin/tar" : "tar", ["-xzf"]);
  // A download made by fetch isn't quarantined, but clear it anyway so Gatekeeper never stops whisper-cli.
  if (process.platform === "darwin") await succeeds("/usr/bin/xattr", ["-dr", "com.apple.quarantine", home]);
  return { cli, device: build.device };
}

const MISSING_WHISPER = "Install whisper.cpp so `whisper-cli` is on PATH, or set whisperCli in linker-edit.config.json.";

/** The ffmpeg Linker-Edit ships, or one on PATH. */
export function findFfmpeg(): string {
  // ffmpeg-static downloads its binary in an install script, which npm can be set to skip.
  const ffmpeg = ffmpegPath && fs.existsSync(ffmpegPath) ? ffmpegPath : findOnPath("ffmpeg");
  if (!ffmpeg) throw new Error("ffmpeg is missing. Reinstall Linker-Edit, or install ffmpeg so it's on your PATH.");
  return ffmpeg;
}

/** Is everything prepareTools needs already here, so a first transcription starts without downloading? */
export function toolsReady(dir: string, override: string | undefined): boolean {
  try {
    findFfmpeg();
  } catch {
    return false;
  }
  const bundled = (["cuda", "cpu"] as const).map((kind) => path.join(dir, `whisper.cpp-${WHISPER_BUILD}-${kind}`, "Release", "whisper-cli.exe"));
  const downloaded = tarWhisperCli(dir);
  const whisper = override
    ? fs.existsSync(override)
    : process.platform === "win32" && process.arch === "x64"
      ? bundled.some((file) => fs.existsSync(file))
      : findOnPath("whisper-cli") !== null || (downloaded !== null && fs.existsSync(downloaded));
  return whisper && [MODEL, VAD_MODEL].every((item) => fs.existsSync(path.join(dir, "models", path.basename(item.url))));
}

/** Finds or downloads ffmpeg, whisper.cpp and its models. The first run downloads up to 1.6 GB. */
export async function prepareTools(dir: string, override: string | undefined, onProgress: SetupProgress, signal?: AbortSignal): Promise<WhisperTools> {
  const ffmpeg = findFfmpeg();
  let whisperCli = override ?? null;
  let device = "";
  if (!whisperCli && process.platform === "win32" && process.arch === "x64") {
    const bundled = await bundledWindowsWhisper(dir, onProgress, signal);
    whisperCli = bundled.cli;
    device = bundled.gpu ? " (CUDA)" : " (CPU)";
  }
  whisperCli ??= findOnPath("whisper-cli");
  if (!whisperCli) {
    const bundled = await bundledTarWhisper(dir, onProgress, signal);
    if (bundled) {
      whisperCli = bundled.cli;
      device = ` (${bundled.device})`;
    }
  }
  if (!whisperCli || !fs.existsSync(whisperCli)) throw new Error(`whisper.cpp not found. ${MISSING_WHISPER}`);

  const model = path.join(dir, "models", path.basename(MODEL.url));
  const vadModel = path.join(dir, "models", path.basename(VAD_MODEL.url));
  await download(MODEL, model, (percent) => onProgress("Downloading the speech model", percent), signal);
  await download(VAD_MODEL, vadModel, (percent) => onProgress("Downloading the voice detector", percent), signal);
  return { ffmpeg, whisperCli, model, vadModel, label: `whisper.cpp ${MODEL_NAME}${device}` };
}
