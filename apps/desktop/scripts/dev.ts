// `npm run dev`: the browser UI from Vite at http://localhost:5317, which hot-reloads, in front of the Rust server
// on 4317 (vite.config.ts sends /ws, /api and media there). With cargo-watch installed (`cargo install
// cargo-watch`), the server is rebuilt and restarted when Rust code changes; without it, restart this.
// Ctrl+C stops both.
import { spawn, spawnSync, type ChildProcess } from "node:child_process";
import * as path from "node:path";

const REPO = path.join(import.meta.dirname, "..", "..", "..");
const watch = spawnSync("cargo watch --version", { shell: true, stdio: "ignore" }).status === 0;
const server = watch ? 'cargo watch -w crates -x "run -p linker-edit -- serve"' : "cargo run -p linker-edit -- serve";

const children: ChildProcess[] = [];

function start(name: string, command: string, cwd: string) {
  // Its own process group elsewhere, so stopping it stops what the shell started.
  const child = spawn(command, { cwd, shell: true, detached: process.platform !== "win32", stdio: ["ignore", "pipe", "pipe"] });
  const prefix = `[${name}] `;
  for (const stream of [child.stdout, child.stderr]) {
    let rest = "";
    stream.setEncoding("utf8");
    stream.on("data", (chunk: string) => {
      const lines = (rest + chunk).split("\n");
      rest = lines.pop() ?? "";
      for (const line of lines) console.log(prefix + line);
    });
  }
  child.on("exit", (code) => {
    console.log(`${prefix}stopped (${code ?? "signal"})`);
    stopAll(code ?? 1);
  });
  children.push(child);
}

function stopAll(code: number) {
  for (const child of children) {
    if (child.exitCode !== null || child.pid === undefined) continue;
    // The shell started each one; stop its whole tree.
    if (process.platform === "win32") spawnSync("taskkill", ["/pid", String(child.pid), "/T", "/F"], { stdio: "ignore" });
    else process.kill(-child.pid, "SIGTERM");
  }
  process.exit(code);
}

process.on("SIGINT", () => stopAll(0));
process.on("SIGTERM", () => stopAll(0));

if (!watch) console.log("[dev] Install cargo-watch to restart the server when Rust code changes.");
start("server", server, REPO);
start("web", "npx vite dev", path.join(REPO, "apps", "desktop"));
