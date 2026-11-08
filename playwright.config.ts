import { defineConfig } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// The e2e server gets its own port and workspace, and replays a recorded Claude turn: no tokens spent.
const port = 4399;
const dir = path.join(os.tmpdir(), "linker-edit-e2e");
const configFile = path.join(dir, "linker-edit.config.json");
// Fresh each run: the server keeps the chat in the workspace across restarts.
fs.rmSync(dir, { recursive: true, force: true });
fs.mkdirSync(dir, { recursive: true });
fs.writeFileSync(configFile, JSON.stringify({ port, workspaceDir: path.join(dir, "workspace") }));

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
