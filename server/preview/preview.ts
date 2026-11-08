import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as path from "node:path";
import { createInterface } from "node:readline";
import type { ResolveMcpConfig } from "../config.ts";

/** What render.py prints, one per line. */
export type PreviewEvent =
  | { event: "start"; timeline: { id: string; name: string } }
  | { event: "progress"; percent: number }
  | { event: "done"; file: string }
  | { event: "error"; message: string; stopped?: boolean };

export interface PreviewRenderer {
  /** Renders a timeline, or Resolve's current one, calling back until a done or error event. */
  render(timelineId: string | null, onEvent: (event: PreviewEvent) => void): { stop(): void };
  /** The rendered preview with this file name, or null. */
  file(name: string): string | null;
}

const SCRIPT = path.join(import.meta.dirname, "render.py");
const PREVIEW_FILE = /^preview-\d+\.mp4$/;

export function createPreviewRenderer(options: { dir: string; mcp: ResolveMcpConfig }): PreviewRenderer {
  const { dir, mcp } = options;
  return {
    render(timelineId, onEvent) {
      // One preview at a time: the last one goes once a new render starts.
      fs.mkdirSync(dir, { recursive: true });
      for (const name of fs.readdirSync(dir)) {
        if (PREVIEW_FILE.test(name)) fs.rmSync(path.join(dir, name), { force: true });
      }
      const child = spawn(mcp.python, [SCRIPT, dir, `preview-${Date.now()}`, ...(timelineId ? [timelineId] : [])], {
        windowsHide: true,
        env: { ...process.env, ...mcp.env },
      });
      let finished = false;
      const finish = (event: PreviewEvent) => {
        if (finished) return;
        if (event.event === "done" || event.event === "error") finished = true;
        onEvent(event);
      };
      createInterface({ input: child.stdout }).on("line", (line) => {
        try {
          finish(JSON.parse(line) as PreviewEvent);
        } catch {
          // Resolve's scripting module prints warnings of its own.
        }
      });
      child.on("error", (error) => finish({ event: "error", message: error.message }));
      child.on("close", () => finish({ event: "error", message: "The preview render stopped without a result." }));
      return {
        stop() {
          if (!finished) child.stdin.write("stop\n");
        },
      };
    },
    file(name) {
      const file = path.join(dir, name);
      return PREVIEW_FILE.test(name) && fs.existsSync(file) ? file : null;
    },
  };
}
