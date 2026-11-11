// Builds the desktop installer for this computer's platform into release/: an NSIS installer on Windows, a dmg
// and zip on macOS, an AppImage on Linux. Run `npm run build` first; `npm run dist:desktop` does both.
//
//   node scripts/build-desktop.ts [--dir]
//
// --dir only lays out the unpacked app (release/<platform>-unpacked), which is quicker and what CI starts.
//
// The app is staged in .desktop-stage with its own package.json and only the packages it runs with: electron is a
// dependency of the npm package, which starts it, and electron-builder refuses to package it as one.
import { spawnSync } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { Arch, build, Platform, type Configuration } from "electron-builder";

const ROOT = path.join(import.meta.dirname, "..");
const STAGE = path.join(ROOT, ".desktop-stage");
const OUT = path.join(ROOT, "release");

type Pkg = {
  name: string;
  version: string;
  description: string;
  license: string;
  repository: { url: string };
  dependencies: Record<string, string>;
  allowScripts?: Record<string, boolean>;
};

const readJson = <T>(file: string) => JSON.parse(fs.readFileSync(file, "utf8")) as T;
const pkg = readJson<Pkg>(path.join(ROOT, "package.json"));
const electronVersion = readJson<{ version: string }>(path.join(ROOT, "node_modules", "electron", "package.json")).version;
const [owner, repo] = new URL(pkg.repository.url).pathname.replace(/^\//, "").replace(/\.git$/, "").split("/");

for (const built of ["build/index.html", "dist/desktop.js", "dist/linker-edit.js"]) {
  if (!fs.existsSync(path.join(ROOT, built))) {
    console.error(`${built} is missing. Run \`npm run build\` first.`);
    process.exit(1);
  }
}

console.log(`Staging Linker-Edit ${pkg.version} in ${STAGE}`);
fs.rmSync(STAGE, { recursive: true, force: true });
fs.rmSync(OUT, { recursive: true, force: true });
const files = ["build", "dist", "desktop/preload.cjs", "NOTICE", "LICENSE", ...fs.globSync("server/resolve/scripts/*.py", { cwd: ROOT })];
for (const file of files) fs.cpSync(path.join(ROOT, file), path.join(STAGE, file), { recursive: true });

const { electron: _, ...dependencies } = pkg.dependencies;
const stagePkg = {
  name: pkg.name,
  productName: "Linker-Edit",
  version: pkg.version,
  description: pkg.description,
  license: pkg.license,
  author: "Linker-Edit contributors",
  homepage: `https://github.com/${owner}/${repo}`,
  type: "module",
  main: "dist/desktop.js",
  dependencies,
  allowScripts: pkg.allowScripts,
};
fs.writeFileSync(path.join(STAGE, "package.json"), `${JSON.stringify(stagePkg, null, 2)}\n`);
// The lockfile keeps the staged packages at the versions CI tested.
fs.copyFileSync(path.join(ROOT, "package-lock.json"), path.join(STAGE, "package-lock.json"));
const install = spawnSync("npm", ["install", "--omit=dev", "--no-audit", "--no-fund"], { cwd: STAGE, stdio: "inherit", shell: true });
if (install.status !== 0) process.exit(install.status ?? 1);

// Signing is optional: electron-builder signs when these are set, and an empty secret counts as unset.
const signing = ["CSC_LINK", "CSC_KEY_PASSWORD", "WIN_CSC_LINK", "WIN_CSC_KEY_PASSWORD", "APPLE_ID", "APPLE_APP_SPECIFIC_PASSWORD", "APPLE_TEAM_ID"];
for (const name of signing) if (!process.env[name]) delete process.env[name];
const macSigned = Boolean(process.env.CSC_LINK);

const config: Configuration = {
  appId: "com.ferretmedia.linkeredit",
  productName: "Linker-Edit",
  copyright: "Copyright © 2026 Linker-Edit contributors",
  artifactName: "Linker-Edit-${version}-${os}-${arch}.${ext}",
  electronVersion,
  // The agents' own programs (claude, codex, ffmpeg) and the Python scripts must be real files to run.
  asar: false,
  npmRebuild: false,
  directories: { output: OUT, buildResources: path.join(ROOT, "desktop", "resources") },
  files: ["**/*", "!package-lock.json"],
  // Where the app looks for updates. Uploading is the release workflow's job, so the build never publishes.
  publish: { provider: "github", owner, repo },
  win: { target: "nsis" },
  nsis: { oneClick: true, perMachine: false },
  mac: {
    target: ["dmg", "zip"],
    category: "public.app-category.video",
    // Without a Developer ID, sign ad hoc so Apple silicon runs the app at all.
    identity: macSigned ? undefined : "-",
    hardenedRuntime: macSigned,
  },
  linux: { target: "AppImage", executableName: "linker-edit", category: "AudioVideo" },
};

const dirOnly = process.argv.includes("--dir");
const arch = process.arch === "arm64" ? Arch.arm64 : Arch.x64;
await build({
  projectDir: STAGE,
  config,
  publish: "never",
  targets: Platform.current().createTarget(dirOnly ? "dir" : undefined, arch),
});
console.log(`Built into ${OUT}`);
