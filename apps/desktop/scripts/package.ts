// Packages a release build, after `npm run build` and `cargo build --release`. Each step writes into release/ at
// the repository root:
//
//   node scripts/package.ts app [--target <t>] [--binary <file>] [--bundle <folder>]
//       release/app-<t>/: the binary with claude, codex/ and ffmpeg next to it (scripts/bundle.ts; --bundle copies
//       a folder bundle.ts already filled instead of downloading), LICENSE, NOTICE and the example config.
//   node scripts/package.ts installer [--target <t>] [--notes <file>] [-- <more vpk options>]
//       release/velopack/: the installer and update packages Velopack's `vpk pack` makes from app-<t>, for the
//       channel <os>-<arch>. $VPK is the vpk to run ("vpk" on PATH by default). The release workflow passes the
//       signing options after `--` when the secrets are there.
//   node scripts/package.ts npm [--target <t>]
//       release/npm/: @linker-edit/<t>, the npm package with app-<t> in bin/.
//   node scripts/package.ts launcher
//       release/npm/: linker-edit, the npm package `npx linker-edit` runs; it starts its platform package's binary.
//   node scripts/package.ts bridge <release assets>
//       latest-mac.yml and latest-linux.yml for the Electron apps before Velopack: see RELEASING.md.
//
// The version is the Cargo workspace's, which scripts/stamp-version.ts sets. <t> is this computer's by default.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { parseArgs } from "node:util";
import { bundle, currentTarget, TARGETS, type Target } from "./bundle.ts";

const REPO = path.join(import.meta.dirname, "..", "..", "..");
const OUT = path.join(REPO, "release");
const PACK_ID = "Linker-Edit";
const BUNDLE_ID = "com.ferretmedia.linkeredit";
const REPOSITORY = { type: "git", url: "git+https://github.com/FerretMedia/linker-edit.git" };

export function workspaceVersion(): string {
  const cargo = fs.readFileSync(path.join(REPO, "Cargo.toml"), "utf8");
  const match = /\[workspace\.package\][^[]*?\nversion = "([^"]+)"/.exec(cargo);
  if (!match) throw new Error("Cargo.toml has no [workspace.package] version.");
  return match[1]!;
}

/** Velopack's name for a target's channel and runtime: win-x64, osx-arm64, linux-x64. */
export function channel(target: Target): string {
  const [platform, arch] = target.split("-") as [string, string];
  return `${{ win32: "win", darwin: "osx", linux: "linux" }[platform]}-${arch}`;
}

const exe = (target: Target) => (target.startsWith("win32") ? ".exe" : "");

function run(command: string, args: string[], cwd: string) {
  console.log(`> ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, { cwd, stdio: "inherit" });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

/** `npm pack` in dir, into release/npm. */
function npmPack(dir: string) {
  fs.mkdirSync(path.join(OUT, "npm"), { recursive: true });
  // Through the shell, which finds npm.cmd on Windows; the path has no quotes in it.
  const result = spawnSync(`npm pack --pack-destination "${path.join(OUT, "npm")}"`, { cwd: dir, stdio: "inherit", shell: true });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

async function app(target: Target, binary: string | undefined, from: string | undefined) {
  const dir = path.join(OUT, `app-${target}`);
  const file = binary ?? path.join(REPO, "target", "release", `linker-edit${exe(target)}`);
  if (!fs.existsSync(file)) throw new Error(`${file} is missing. Run \`cargo build --release -p linker-edit\` first.`);
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  if (from) fs.cpSync(from, dir, { recursive: true });
  else await bundle(dir, target);
  fs.copyFileSync(file, path.join(dir, `linker-edit${exe(target)}`));
  fs.chmodSync(path.join(dir, `linker-edit${exe(target)}`), 0o755);
  for (const name of ["LICENSE", "NOTICE", "linker-edit.config.example.json"]) fs.copyFileSync(path.join(REPO, name), path.join(dir, name));
  console.log(`Laid out ${dir}`);
}

function installer(target: Target, notes: string | undefined, extra: string[]) {
  const dir = path.join(OUT, `app-${target}`);
  if (!fs.existsSync(dir)) throw new Error(`${dir} is missing. Run \`node scripts/package.ts app\` first.`);
  const icons = path.join(REPO, "crates", "app", "icons");
  const icon = target.startsWith("win32") ? "icon.ico" : target.startsWith("darwin") ? "icon.icns" : "icon.png";
  const args = [
    "pack",
    "--packId", PACK_ID,
    "--packVersion", workspaceVersion(),
    "--packTitle", "Linker-Edit",
    "--packAuthors", "Linker-Edit contributors",
    "--packDir", dir,
    "--mainExe", `linker-edit${exe(target)}`,
    "--channel", channel(target),
    "--runtime", channel(target),
    "--icon", path.join(icons, icon),
    "--outputDir", path.join(OUT, "velopack"),
  ];
  if (target.startsWith("darwin")) args.push("--bundleId", BUNDLE_ID);
  if (target.startsWith("linux")) args.push("--categories", "AudioVideo");
  if (notes) args.push("--releaseNotes", path.resolve(notes));
  run(process.env.VPK || "vpk", [...args, ...extra], REPO);
}

function npmPlatform(target: Target) {
  const app = path.join(OUT, `app-${target}`);
  if (!fs.existsSync(app)) throw new Error(`${app} is missing. Run \`node scripts/package.ts app\` first.`);
  const stage = path.join(OUT, "stage", target);
  const [os, cpu] = target.split("-") as [string, string];
  const version = workspaceVersion();
  fs.rmSync(stage, { recursive: true, force: true });
  fs.cpSync(app, path.join(stage, "bin"), { recursive: true });
  for (const name of ["LICENSE", "NOTICE"]) fs.renameSync(path.join(stage, "bin", name), path.join(stage, name));
  const pkg = {
    name: `@linker-edit/${target}`,
    version,
    description: `Linker-Edit for ${target}: the program \`npx linker-edit\` runs, with Claude Code, Codex and ffmpeg.`,
    license: "MIT",
    repository: REPOSITORY,
    os: [os],
    cpu: [cpu],
    files: ["bin", "NOTICE"],
    preferUnplugged: true,
  };
  fs.writeFileSync(path.join(stage, "package.json"), `${JSON.stringify(pkg, null, 2)}\n`);
  fs.writeFileSync(
    path.join(stage, "README.md"),
    `# @linker-edit/${target}\n\nThe Linker-Edit program for ${target}. Install and run [linker-edit](https://www.npmjs.com/package/linker-edit) instead: \`npx linker-edit@latest\`.\n`,
  );
  npmPack(stage);
}

function launcher() {
  const stage = path.join(OUT, "stage", "linker-edit");
  const version = workspaceVersion();
  fs.rmSync(stage, { recursive: true, force: true });
  fs.cpSync(path.join(REPO, "npm", "linker-edit"), stage, { recursive: true });
  for (const name of ["README.md", "LICENSE", "NOTICE"]) fs.copyFileSync(path.join(REPO, name), path.join(stage, name));
  const file = path.join(stage, "package.json");
  const pkg = JSON.parse(fs.readFileSync(file, "utf8")) as Record<string, unknown>;
  pkg.version = version;
  pkg.optionalDependencies = Object.fromEntries(TARGETS.map((target) => [`@linker-edit/${target}`, version]));
  fs.writeFileSync(file, `${JSON.stringify(pkg, null, 2)}\n`);
  npmPack(stage);
}

/**
 * The update manifests electron-updater reads, for the Electron apps from before Velopack (v2026.11.14 and later).
 * A Mac app only shows the new version with a link to download it, as it always did; a Linux AppImage downloads the
 * new one and swaps itself for it. Windows gets none: its installer would put Velopack's install next to the old one.
 */
export function bridgeManifests(files: string[], read: (file: string) => Buffer, version: string, date: string): Map<string, string> {
  const manifest = (list: string[]) => {
    const entries = list.map((file) => {
      const data = read(file);
      return { url: path.basename(file), sha512: createHash("sha512").update(data).digest("base64"), size: data.length };
    });
    return [
      `version: ${version}`,
      "files:",
      ...entries.flatMap((entry) => [`  - url: ${entry.url}`, `    sha512: ${entry.sha512}`, `    size: ${entry.size}`]),
      `path: ${entries[0]!.url}`,
      `sha512: ${entries[0]!.sha512}`,
      `releaseDate: '${date}'`,
      "",
    ].join("\n");
  };
  const manifests = new Map<string, string>();
  const mac = files.filter((file) => /-osx-(arm64|x64)-Portable\.zip$/.test(file)).sort();
  if (mac.length > 0) manifests.set("latest-mac.yml", manifest(mac));
  const linux = files.filter((file) => /-linux-x64\.AppImage$/.test(file));
  if (linux.length > 0) manifests.set("latest-linux.yml", manifest(linux));
  return manifests;
}

function bridge(dir: string) {
  const files = fs.readdirSync(dir).map((name) => path.join(dir, name));
  const manifests = bridgeManifests(files, (file) => fs.readFileSync(file), workspaceVersion(), new Date().toISOString());
  for (const [name, text] of manifests) {
    fs.writeFileSync(path.join(dir, name), text);
    console.log(`Wrote ${name}`);
  }
}

if (import.meta.main) {
  const { values, positionals } = parseArgs({
    options: { target: { type: "string" }, binary: { type: "string" }, bundle: { type: "string" }, notes: { type: "string" } },
    allowPositionals: true,
  });
  const [step, dir, ...rest] = positionals;
  if (values.target && !(TARGETS as readonly string[]).includes(values.target)) {
    console.error(`--target is one of ${TARGETS.join(", ")}`);
    process.exit(1);
  }
  const target = (values.target as Target | undefined) ?? currentTarget();
  switch (step) {
    case "app":
      await app(target, values.binary, values.bundle);
      break;
    case "installer":
      installer(target, values.notes, dir === undefined ? [] : [dir, ...rest]);
      break;
    case "npm":
      npmPlatform(target);
      break;
    case "launcher":
      launcher();
      break;
    case "bridge":
      if (!dir) process.exit(1);
      bridge(dir);
      break;
    default:
      console.error("Usage: node scripts/package.ts <app|installer|npm|launcher|bridge> [options]; see the top of this file.");
      process.exit(1);
  }
}
