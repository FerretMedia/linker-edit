import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { ResolveError } from "../resolve/connection.ts";
import { fakeConnection } from "../resolve/testing.ts";
import { createPreviewRenderer, type PreviewEvent } from "./preview.ts";

const timeline = { id: "t1", name: "AI - Cut" };

function render(script: (name: string, args: unknown) => unknown, stopAfterStart = false) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-preview-"));
  const { connection, scripts } = fakeConnection({ script: (name, args) => script(name, { ...(args as object), dir }) });
  const renderer = createPreviewRenderer({ dir, connection, pollMs: 1 });
  const events: PreviewEvent[] = [];
  return new Promise<{ events: PreviewEvent[]; scripts: string[]; dir: string }>((resolve) => {
    const job = renderer.render("t1", (event) => {
      events.push(event);
      if (event.event === "start" && stopAfterStart) job.stop();
      if (event.event === "done" || event.event === "error") resolve({ events, scripts: scripts.map((call) => call.name), dir });
    });
  });
}

const started = (args: unknown) => {
  const { dir, name } = args as { dir: string; name: string };
  return { timeline, target_dir: dir, file_name: `${name}.mp4`, restore: { job: "j1", page: "edit" } };
};

describe("createPreviewRenderer", () => {
  it("starts, reports progress, finds the file and puts Resolve back", async () => {
    let polls = 0;
    let file = "";
    const result = await render((name, args) => {
      if (name === "preview_start") {
        const out = started(args);
        file = path.join(out.target_dir, out.file_name);
        return out;
      }
      if (name === "preview_poll") {
        polls += 1;
        if (polls < 3) return { rendering: true, percent: polls * 40, status: "Rendering" };
        fs.writeFileSync(file, "mp4");
        return { rendering: false, percent: 100, status: "Complete" };
      }
      return true;
    });
    expect(result.events).toEqual([
      { event: "start", timeline },
      { event: "progress", percent: 40 },
      { event: "progress", percent: 80 },
      { event: "done", file },
    ]);
    expect(result.scripts.at(-1)).toBe("preview_finish");
  });

  it("stops, removes the partial file and still puts Resolve back", async () => {
    const result = await render((name, args) => (name === "preview_start" ? started(args) : name === "preview_poll" ? { rendering: true, percent: 5, status: "Rendering" } : true), true);
    expect(result.events.at(-1)).toEqual({ event: "error", message: "Preview stopped.", stopped: true });
    expect(result.scripts).toContain("preview_stop");
    expect(result.scripts.at(-1)).toBe("preview_finish");
  });

  it("passes on why Resolve couldn't start", async () => {
    const result = await render(() => {
      throw new ResolveError("refused", "Resolve has no timeline open.");
    });
    expect(result.events).toEqual([{ event: "error", message: "Resolve has no timeline open." }]);
    expect(result.scripts).toEqual(["preview_start"]);
  });

  it("reports a render that didn't complete", async () => {
    const result = await render((name, args) => (name === "preview_start" ? started(args) : name === "preview_poll" ? { rendering: false, percent: 20, status: "Failed" } : true));
    expect(result.events.at(-1)).toEqual({ event: "error", message: "Resolve's render ended with status Failed." });
  });
});
