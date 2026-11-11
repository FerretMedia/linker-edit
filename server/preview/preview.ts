import * as fs from "node:fs";
import * as path from "node:path";
import type { ResolveConnection } from "../resolve/connection.ts";

/** What a render reports as it goes. */
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

const PREVIEW_FILE = /^preview-\d+\.mp4$/;

interface Started {
  timeline: { id: string; name: string };
  target_dir: string;
  file_name: string;
  /** What preview_finish.py needs to put Resolve back as it was. */
  restore: Record<string, unknown> & { job: string };
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/**
 * Renders through Resolve in steps, each a script of its own (Resolve's MCP stops scripts after 60
 * seconds): start, poll until done or stopped, then put Resolve back as it was.
 */
export function createPreviewRenderer(options: {
  dir: string;
  connection: ResolveConnection;
  pollMs?: number;
}): PreviewRenderer {
  const { dir, connection, pollMs = 500 } = options;
  return {
    render(timelineId, onEvent) {
      let stopping = false;
      void (async () => {
        let started: Started;
        try {
          // One preview at a time: the last one goes once a new render starts.
          fs.mkdirSync(dir, { recursive: true });
          for (const name of fs.readdirSync(dir)) {
            if (PREVIEW_FILE.test(name)) fs.rmSync(path.join(dir, name), { force: true });
          }
          started = await connection.runScript<Started>("preview_start", { dir, name: `preview-${Date.now()}`, timeline_id: timelineId }, 30);
        } catch (error) {
          onEvent({ event: "error", message: errorText(error) });
          return;
        }
        onEvent({ event: "start", timeline: started.timeline });
        const file = path.join(started.target_dir, started.file_name);
        let result: PreviewEvent;
        try {
          let last = -1;
          let status: string | null = null;
          for (;;) {
            if (stopping) {
              await connection.runScript("preview_stop", {}, 45);
              break;
            }
            const poll = await connection.runScript<{ rendering: boolean; percent: number; status: string | null }>("preview_poll", { job: started.restore.job }, 10);
            status = poll.status;
            if (!poll.rendering) break;
            if (poll.percent !== last) onEvent({ event: "progress", percent: (last = poll.percent) });
            await new Promise((resolve) => setTimeout(resolve, pollMs));
          }
          if (stopping) {
            fs.rmSync(file, { force: true });
            result = { event: "error", message: "Preview stopped.", stopped: true };
          } else if (status !== "Complete" || !fs.existsSync(file)) {
            result = { event: "error", message: `Resolve's render ended with status ${status ?? "unknown"}.` };
          } else {
            result = { event: "done", file };
          }
        } catch (error) {
          result = { event: "error", message: errorText(error) };
        }
        // Only once Resolve is put back may the app let an agent at it again.
        try {
          await connection.runScript("preview_finish", { restore: started.restore }, 30);
        } catch (error) {
          console.error(`Couldn't put Resolve back after the preview: ${errorText(error)}`);
        }
        onEvent(result);
      })();
      return {
        stop() {
          stopping = true;
        },
      };
    },
    file(name) {
      const file = path.join(dir, name);
      return PREVIEW_FILE.test(name) && fs.existsSync(file) ? file : null;
    },
  };
}
