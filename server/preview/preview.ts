import * as fs from "node:fs";
import * as path from "node:path";
import type { ResolveConnection } from "../resolve/connection.ts";
import { renderTimeline } from "../resolve/render.ts";

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

/** A small MP4 of a timeline for the preview panel, rendered by Resolve. */
export function createPreviewRenderer(options: { dir: string; connection: ResolveConnection; pollMs?: number }): PreviewRenderer {
  const { dir, connection, pollMs = 500 } = options;
  return {
    render(timelineId, onEvent) {
      try {
        // One preview at a time: the last one goes once a new render starts.
        fs.mkdirSync(dir, { recursive: true });
        for (const name of fs.readdirSync(dir)) {
          if (PREVIEW_FILE.test(name)) fs.rmSync(path.join(dir, name), { force: true });
        }
      } catch (error) {
        queueMicrotask(() => onEvent({ event: "error", message: error instanceof Error ? error.message : String(error) }));
        return { stop() {} };
      }
      const job = renderTimeline(
        connection,
        { timelineId, dir, name: `preview-${Date.now()}`, format: "mp4", maxWidth: 960, audio: true, video: true },
        (event) => {
          if (event.event === "done") onEvent({ event: "done", file: event.file });
          else if (event.event === "error" && event.stopped) onEvent({ event: "error", message: "Preview stopped.", stopped: true });
          else onEvent(event);
        },
        pollMs,
      );
      return { stop: job.stop };
    },
    file(name) {
      const file = path.join(dir, name);
      return PREVIEW_FILE.test(name) && fs.existsSync(file) ? file : null;
    },
  };
}
