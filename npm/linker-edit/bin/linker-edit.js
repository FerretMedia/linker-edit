#!/usr/bin/env node
// `npx linker-edit`: runs the Linker-Edit binary from this computer's platform package (@linker-edit/<platform>),
// which npm installed as an optional dependency, with the same arguments, the same terminal and its exit code.
// Without arguments it opens Linker-Edit in the browser (`--browser`): the desktop app is the installer from the
// GitHub release, and a computer reached over npm may have no display. `npx linker-edit window` opens the window.
import { spawn } from "node:child_process";
import * as fs from "node:fs";
import { createRequire } from "node:module";
import * as path from "node:path";

const PLATFORMS = ["win32-x64", "darwin-arm64", "darwin-x64", "linux-x64"];
const platform = `${process.platform}-${process.arch}`;

function fail(message) {
  console.error(message);
  process.exit(1);
}

if (!PLATFORMS.includes(platform)) {
  fail(`Linker-Edit isn't built for ${platform}. It runs on ${PLATFORMS.join(", ")}.`);
}

let dir;
try {
  dir = path.dirname(createRequire(import.meta.url).resolve(`@linker-edit/${platform}/package.json`));
} catch {
  fail(
    `Linker-Edit's program for ${platform} (@linker-edit/${platform}) isn't installed. npm installs it with ` +
      "linker-edit unless optional dependencies are turned off (--omit=optional): install again without that.",
  );
}

const bin = path.join(dir, "bin");
const binary = path.join(bin, process.platform === "win32" ? "linker-edit.exe" : "linker-edit");
// The programs have to be executable; a package manager that dropped the bit gets it back where it can.
if (process.platform !== "win32") {
  for (const name of ["linker-edit", "claude", "ffmpeg", "codex/bin/codex"]) {
    try {
      const file = path.join(bin, name);
      const { mode } = fs.statSync(file);
      if ((mode & 0o111) !== 0o111) fs.chmodSync(file, mode | 0o755);
    } catch {
      // Read-only installs keep what they have.
    }
  }
}

const args = process.argv.slice(2);
const child = spawn(binary, args.length > 0 ? args : ["--browser"], { stdio: "inherit" });
child.on("error", (error) => fail(`Couldn't start ${binary}: ${error.message}`));

// Ctrl+C reaches Linker-Edit too, which stops by itself; this one waits for it and exits as it did.
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => {
    if (signal !== "SIGINT") child.kill(signal);
  });
}
child.on("exit", (code, signal) => {
  if (signal) {
    process.removeAllListeners(signal);
    process.kill(process.pid, signal);
  } else {
    process.exit(code ?? 1);
  }
});
