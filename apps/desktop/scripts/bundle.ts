// Fetches the programs that ship next to the linker-edit binary, from npm, at the versions the code pins:
//
//   claude(.exe)   Claude Code's native binary from @anthropic-ai/claude-agent-sdk-<platform>, at the Agent SDK
//                  version crates/server/src/providers/cli.rs names
//   codex/         @openai/codex's platform build, its vendor/<target triple> folder unchanged (cli.rs's version)
//   ffmpeg(.exe)   the build ffmpeg-static (the version in package-lock.json) downloads, with its LICENSE and README
//
//   node scripts/bundle.ts <folder> [--target win32-x64|darwin-arm64|darwin-x64|linux-x64]
//
// The target is this computer's by default. Nothing runs from the packages: they are only unpacked.
import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { parseArgs } from "node:util";
import * as zlib from "node:zlib";

export const TARGETS = ["win32-x64", "darwin-arm64", "darwin-x64", "linux-x64"] as const;
export type Target = (typeof TARGETS)[number];

const REPO = path.join(import.meta.dirname, "..", "..", "..");

export function currentTarget(): Target {
  const target = `${process.platform}-${process.arch}`;
  if (!(TARGETS as readonly string[]).includes(target)) throw new Error(`Linker-Edit isn't built for ${target}.`);
  return target as Target;
}

/** The versions the Rust code pins, and the ffmpeg-static the checkout installed. */
export function pinnedVersions(): { agentSdk: string; codex: string; ffmpegStatic: string } {
  const cli = fs.readFileSync(path.join(REPO, "crates", "server", "src", "providers", "cli.rs"), "utf8");
  const constant = (name: string) => {
    const match = new RegExp(`pub const ${name}: &str = "([^"]+)";`).exec(cli);
    if (!match) throw new Error(`cli.rs has no ${name}.`);
    return match[1]!;
  };
  const lock = JSON.parse(fs.readFileSync(path.join(REPO, "package-lock.json"), "utf8")) as { packages: Record<string, { version?: string }> };
  const ffmpegStatic = lock.packages["node_modules/ffmpeg-static"]?.version;
  if (!ffmpegStatic) throw new Error("package-lock.json has no ffmpeg-static.");
  return { agentSdk: constant("AGENT_SDK_VERSION"), codex: constant("CODEX_VERSION"), ffmpegStatic };
}

/** A file in a tarball. */
interface Entry {
  name: string;
  mode: number;
  data: Buffer;
}

/** The files in a .tgz: plain ustar, with the pax headers npm writes for long names. */
export function untar(tgz: Buffer): Entry[] {
  const tar = zlib.gunzipSync(tgz);
  const entries: Entry[] = [];
  let longName: string | null = null;
  for (let at = 0; at + 512 <= tar.length; ) {
    const header = tar.subarray(at, at + 512);
    if (header.every((byte) => byte === 0)) break;
    const text = (start: number, length: number) => header.toString("utf8", start, start + length).replace(/\0.*$/s, "");
    const size = parseInt(text(124, 12).trim() || "0", 8);
    const type = text(156, 1) || "0";
    const prefix = text(345, 155);
    const name = longName ?? (prefix ? `${prefix}/${text(0, 100)}` : text(0, 100));
    const data = tar.subarray(at + 512, at + 512 + size);
    longName = null;
    if (type === "x") {
      const pathRecord = /\d+ path=([^\n]*)\n/.exec(data.toString("utf8"));
      if (pathRecord) longName = pathRecord[1]!;
    } else if (type === "0" || type === "\0") {
      entries.push({ name, mode: parseInt(text(100, 8).trim() || "644", 8), data: Buffer.from(data) });
    }
    at += 512 + Math.ceil(size / 512) * 512;
  }
  return entries;
}

/** Downloads a package's tarball with `npm pack`, which runs none of its scripts. */
function npmPack(spec: string, dir: string): Buffer {
  // One command line through the shell, which finds npm.cmd on Windows; spec and dir have no quotes in them.
  const result = spawnSync(`npm pack ${spec} --json --pack-destination "${dir}"`, { encoding: "utf8", shell: true });
  if (result.status !== 0) throw new Error(`npm pack ${spec} failed:\n${result.stderr}`);
  const [{ filename }] = JSON.parse(result.stdout) as [{ filename: string }];
  return fs.readFileSync(path.join(dir, path.basename(filename)));
}

function write(file: string, data: Buffer, mode: number) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, data);
  fs.chmodSync(file, mode & 0o777);
}

async function download(url: string): Promise<Buffer> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: ${response.status} ${response.statusText}`);
  return Buffer.from(await response.arrayBuffer());
}

/** Puts claude, codex/ and ffmpeg for target into dir. */
export async function bundle(dir: string, target: Target): Promise<void> {
  const versions = pinnedVersions();
  const exe = target.startsWith("win32") ? ".exe" : "";
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-bundle-"));
  try {
    fs.mkdirSync(dir, { recursive: true });

    const claudeName = `package/claude${exe}`;
    const claude = untar(npmPack(`@anthropic-ai/claude-agent-sdk-${target}@${versions.agentSdk}`, tmp)).find((entry) => entry.name === claudeName);
    if (!claude) throw new Error(`@anthropic-ai/claude-agent-sdk-${target} has no ${claudeName}.`);
    write(path.join(dir, `claude${exe}`), claude.data, 0o755);

    // @openai/codex publishes each platform's build as a version of its own: 0.166.0-win32-x64.
    const codex = untar(npmPack(`@openai/codex@${versions.codex}-${target}`, tmp));
    const vendor = /^package\/vendor\/[^/]+\//;
    const files = codex.filter((entry) => vendor.test(entry.name));
    if (!files.some((entry) => entry.name.endsWith(`/bin/codex${exe}`))) throw new Error(`@openai/codex ${versions.codex}-${target} has no bin/codex${exe}.`);
    fs.rmSync(path.join(dir, "codex"), { recursive: true, force: true });
    for (const entry of files) write(path.join(dir, "codex", entry.name.replace(vendor, "")), entry.data, entry.mode);

    // ffmpeg-static's own install script downloads its build from its GitHub release; this does the same for any
    // platform. The release tag is in its package.json.
    const ffmpegPackage = untar(npmPack(`ffmpeg-static@${versions.ffmpegStatic}`, tmp)).find((entry) => entry.name === "package/package.json");
    const tag = (JSON.parse(ffmpegPackage!.data.toString("utf8")) as { "ffmpeg-static": { "binary-release-tag": string } })["ffmpeg-static"]["binary-release-tag"];
    const [platform, arch] = target.split("-") as [string, string];
    const base = `https://github.com/eugeneware/ffmpeg-static/releases/download/${tag}`;
    write(path.join(dir, `ffmpeg${exe}`), zlib.gunzipSync(await download(`${base}/ffmpeg-${platform}-${arch}.gz`)), 0o755);
    write(path.join(dir, `ffmpeg${exe}.LICENSE`), await download(`${base}/${platform}-${arch}.LICENSE`), 0o644);
    write(path.join(dir, `ffmpeg${exe}.README`), await download(`${base}/${platform}-${arch}.README`), 0o644);
    console.log(`Bundled Claude Code (Agent SDK ${versions.agentSdk}), Codex ${versions.codex} and ffmpeg ${tag} for ${target} in ${dir}`);
  } finally {
    fs.rmSync(tmp, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  const { values, positionals } = parseArgs({ options: { target: { type: "string" } }, allowPositionals: true });
  const [dir] = positionals;
  if (!dir || (values.target && !(TARGETS as readonly string[]).includes(values.target))) {
    console.error(`Usage: node scripts/bundle.ts <folder> [--target ${TARGETS.join("|")}]`);
    process.exit(1);
  }
  await bundle(path.resolve(dir), (values.target as Target | undefined) ?? currentTarget());
}
