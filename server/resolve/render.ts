// Rendering a timeline through Resolve, for the preview panel and the agents' render_timeline tool. It runs
// in steps, each a script of its own (Resolve's MCP stops scripts after 60 seconds): start, poll until done
// or stopped, then put Resolve's timeline, page and render settings back as they were.
import * as fs from "node:fs";
import * as path from "node:path";
import type { ResolveConnection } from "./connection.ts";

export interface RenderRequest {
  /** Null renders the timeline open in Resolve. */
  timelineId: string | null;
  dir: string;
  /** File name without extension. Defaults to the timeline's name. */
  name?: string;
  preset?: string;
  format?: string;
  codec?: string;
  width?: number;
  height?: number;
  /** Scales down to at most this width, keeping the aspect ratio. */
  maxWidth?: number;
  fromFrame?: number;
  toFrame?: number;
  audio?: boolean;
  video?: boolean;
}

export type RenderEvent =
  | { event: "start"; timeline: { id: string; name: string } }
  | { event: "progress"; percent: number }
  | { event: "done"; file: string; timeline: { id: string; name: string } }
  | { event: "error"; message: string; stopped?: boolean };

interface Started {
  timeline: { id: string; name: string };
  target_dir: string;
  file_name: string;
  /** What render_finish.py needs to put Resolve back as it was. */
  restore: Record<string, unknown> & { job: string };
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** The rendered file: Resolve names it in the job, adding the extension; look for it when it doesn't. */
function outputFile(started: Started): string | null {
  const named = path.join(started.target_dir, started.file_name);
  if (fs.existsSync(named) && fs.statSync(named).isFile()) return named;
  const stem = path.parse(started.file_name).name;
  if (!fs.existsSync(started.target_dir)) return null;
  const match = fs.readdirSync(started.target_dir).find((file) => path.parse(file).name === stem);
  return match ? path.join(started.target_dir, match) : null;
}

/** Renders one timeline. Events end with "done" or "error"; finished resolves with that last event. */
export function renderTimeline(
  connection: ResolveConnection,
  request: RenderRequest,
  onEvent: (event: RenderEvent) => void = () => {},
  pollMs = 500,
): { stop(): void; finished: Promise<RenderEvent> } {
  let stopping = false;
  const finished = (async (): Promise<RenderEvent> => {
    let started: Started;
    try {
      fs.mkdirSync(request.dir, { recursive: true });
      started = await connection.runScript<Started>(
        "render_start",
        {
          dir: request.dir,
          name: request.name,
          timeline_id: request.timelineId,
          preset: request.preset,
          format: request.format,
          codec: request.codec,
          width: request.width,
          height: request.height,
          max_width: request.maxWidth,
          from_frame: request.fromFrame,
          to_frame: request.toFrame,
          audio: request.audio,
          video: request.video,
        },
        30,
      );
    } catch (error) {
      return { event: "error", message: errorText(error) };
    }
    onEvent({ event: "start", timeline: started.timeline });
    let result: RenderEvent;
    try {
      let last = -1;
      let status: string | null = null;
      for (;;) {
        if (stopping) {
          await connection.runScript("render_stop", {}, 45);
          break;
        }
        const poll = await connection.runScript<{ rendering: boolean; percent: number; status: string | null }>("render_poll", { job: started.restore.job }, 10);
        status = poll.status;
        if (!poll.rendering) break;
        if (poll.percent !== last) onEvent({ event: "progress", percent: (last = poll.percent) });
        await new Promise((resolve) => setTimeout(resolve, pollMs));
      }
      const file = outputFile(started);
      if (stopping) {
        if (file) fs.rmSync(file, { force: true });
        result = { event: "error", message: "Render stopped.", stopped: true };
      } else if (status !== "Complete" || !file) {
        result = { event: "error", message: `Resolve's render ended with status ${status ?? "unknown"}.` };
      } else {
        result = { event: "done", file, timeline: started.timeline };
      }
    } catch (error) {
      result = { event: "error", message: errorText(error) };
    }
    // Only once Resolve is put back may the app let an agent at it again.
    try {
      await connection.runScript("render_finish", { restore: started.restore }, 30);
      const page = started.restore.page;
      // Resolve opens the Deliver page by itself as a render ends, and ignores a switch back made too soon.
      for (let attempt = 0; typeof page === "string" && attempt < 10; attempt += 1) {
        const now = await connection.runScript<{ page: string | null }>("render_page", { page }, 10);
        if (now.page === page && attempt > 0) break;
        await new Promise((resolve) => setTimeout(resolve, pollMs));
      }
    } catch (error) {
      console.error(`Couldn't put Resolve back after rendering: ${errorText(error)}`);
    }
    return result;
  })().then((event) => {
    onEvent(event);
    return event;
  });
  return {
    stop() {
      stopping = true;
    },
    finished,
  };
}
