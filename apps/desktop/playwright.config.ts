import { defineConfig } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// The e2e server gets its own port and workspace, and replays a recorded Claude turn: no tokens spent.
const port = 4399;
const dir = path.join(os.tmpdir(), "linker-edit-e2e");
const configFile = path.join(dir, "linker-edit.config.json");
// Fresh each run: the server keeps its projects and threads across restarts. Only in the main process:
// each test worker loads this file again, while the server is already using the folder.
if (!process.env.TEST_WORKER_INDEX) {
  fs.rmSync(dir, { recursive: true, force: true });
  fs.mkdirSync(dir, { recursive: true });
  // Every folder inside this one, never the user's ~/LinkerEdit; no ResolveMCP, so a Resolve open on this computer
  // is never touched, as on CI; and no Resolve here, so the speech engine isn't downloaded in the background.
  fs.writeFileSync(
    configFile,
    JSON.stringify({
      port,
      workspaceDir: path.join(dir, "workspace"),
      projectsDir: path.join(dir, "projects"),
      previewDir: path.join(dir, "previews"),
      transcriptsDir: path.join(dir, "transcripts"),
      toolsDir: path.join(dir, "tools"),
      resolveMcp: "",
    }),
  );
  fs.writeFileSync(path.join(dir, "settings.json"), JSON.stringify({ resolveHere: false }));
  // A teammate's project folder to add through the folder picker.
  fs.mkdirSync(path.join(dir, "team", "Promo"), { recursive: true });
  fs.writeFileSync(path.join(dir, "team", "Promo", "linker-edit.json"), JSON.stringify({ version: 1, name: "Promo", resolve: null }));
  // A music folder for the library, with one second of a quiet tone.
  fs.mkdirSync(path.join(dir, "music"), { recursive: true });
  fs.writeFileSync(path.join(dir, "music", "Tone.wav"), toneWav());
  // An editor's assets folder, with a folder for each kind of file, for the library to sort.
  for (const [folder, file] of [["Music", "Song.wav"], ["SoundEffects", "Whoosh.wav"], ["VideoFX", "Light leak.mov"], ["Images", "Logo.png"], ["Misc", "notes.txt"]] as const) {
    fs.mkdirSync(path.join(dir, "video-editing", folder), { recursive: true });
    fs.writeFileSync(path.join(dir, "video-editing", folder, file), file.endsWith(".wav") ? toneWav() : "");
  }
}

function toneWav(rate = 8000): Buffer {
  const data = Buffer.alloc(rate * 2);
  for (let i = 0; i < rate; i += 1) data.writeInt16LE(Math.round(Math.sin((2 * Math.PI * 440 * i) / rate) * 8000), i * 2);
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(1, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate * 2, 28);
  header.writeUInt16LE(2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

export default defineConfig({
  testDir: "e2e",
  use: { baseURL: `http://127.0.0.1:${port}` },
  webServer: {
    command: "npm run build && node server/index.ts",
    url: `http://127.0.0.1:${port}`,
    reuseExistingServer: false,
    env: {
      LINKER_EDIT_CONFIG: configFile,
      LINKER_EDIT_REPLAY: path.join("server", "providers", "fixtures", "claude-turn.jsonl"),
    },
  },
});
