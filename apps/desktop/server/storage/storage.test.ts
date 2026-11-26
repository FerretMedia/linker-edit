import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import type { ComputerStorage } from "../../shared/protocol.ts";
import { createStorage } from "./storage.ts";
import { walkFolder } from "./walk.ts";

let dir: string;
beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-storage-"));
});
afterEach(() => fs.rmSync(dir, { recursive: true, force: true }));

function write(file: string, bytes: number) {
  fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
  fs.writeFileSync(path.join(dir, file), Buffer.alloc(bytes));
}

const MEASURED_AT = new Date("2026-11-26T10:00:00.000Z");

describe("measuring this computer", () => {
  it("adds up every file in each project's folder, largest project first, on the disks that hold them", async () => {
    write("promo/a.mov", 1000);
    write("promo/media/b.mov", 2500);
    write("promo/media/proxies/c.mov", 0);
    write("teaser/x.drp", 10);
    const storage = createStorage({ dataDir: dir, file: null, now: () => MEASURED_AT, log: () => {} });
    const report = await storage.measure("pc-1", [
      { projectId: "teaser", name: "Teaser", dir: path.join(dir, "teaser") },
      { projectId: "gone", name: "On an unplugged drive", dir: path.join(dir, "gone") },
      { projectId: "promo", name: "Promo", dir: path.join(dir, "promo") },
    ]);
    expect(report).toMatchObject({
      computerId: "pc-1",
      measuredAt: MEASURED_AT.toISOString(),
      projects: [
        { projectId: "promo", name: "Promo", localBytes: 3500, cloudBytes: null, mirror: null },
        { projectId: "teaser", name: "Teaser", localBytes: 10, cloudBytes: null, mirror: null },
        { projectId: "gone", name: "On an unplugged drive", localBytes: null, cloudBytes: null, mirror: null },
      ],
    });
    // Every folder here is on the temp folder's disk: one disk, with its real numbers.
    expect(report.disks).toHaveLength(1);
    const [disk] = report.disks;
    expect(path.relative(disk!.path, dir).startsWith("..")).toBe(false);
    expect(disk!.totalBytes).toBeGreaterThan(disk!.freeBytes);
    expect(disk!.freeBytes).toBeGreaterThan(0);
  });
});

describe("the folder walk", () => {
  it("reads only folders that changed since the last walk, and still sees files that grew", () => {
    write("a.mov", 1000);
    write("media/b.mov", 2500);
    write("media/proxies/c.mov", 0);
    const first = walkFolder(dir, new Map(), 0);
    expect(first.size).toEqual({ bytes: 3500, files: 3, listed: 3, reused: 0 });

    const unchanged = walkFolder(dir, first.listings, 1000);
    expect(unchanged.size).toEqual({ bytes: 3500, files: 3, listed: 0, reused: 3 });

    // A render still being written grows without touching its folder.
    fs.appendFileSync(path.join(dir, "media/b.mov"), Buffer.alloc(500));
    // A new clip changes its folder, which is read again.
    write("media/proxies/d.mov", 40);
    const changed = walkFolder(dir, unchanged.listings, 2000);
    expect(changed.size).toEqual({ bytes: 4040, files: 4, listed: 1, reused: 2 });

    // A listing kept too long is read again, for disks that leave a folder's mtime alone.
    expect(walkFolder(dir, changed.listings, 2000 + 10 * 60_000).size).toMatchObject({ listed: 3, reused: 0 });
  });
});

describe("other computers' reports", () => {
  const report: ComputerStorage = {
    computerId: "office-id",
    disks: [{ path: "D:\\", label: "D:", totalBytes: 4e12, freeBytes: 1e12 }],
    projects: [{ projectId: "wedding", name: "Wedding film", localBytes: 2e11, cloudBytes: null, mirror: null }],
    measuredAt: "2026-11-25T18:30:00.000Z",
  };
  const answers = (value: unknown) => async () => value;
  const fails = (problem: string) => async () => {
    throw new Error(problem);
  };

  it("shows the last report of a computer that doesn't answer, marked stale, even after a restart", async () => {
    const file = path.join(dir, "storage.json");
    const home = createStorage({ dataDir: dir, file, log: () => {} });
    expect(await home.card("office", answers(report))).toEqual({ state: "fresh", report });
    expect(await home.card("office", fails("Couldn't reach Office."))).toEqual({ state: "stale", report, problem: "Couldn't reach Office." });
    // An answer that isn't a report doesn't replace the last one.
    expect(await home.card("office", answers({ error: "Not found" }))).toMatchObject({ state: "stale", report });

    const restarted = createStorage({ dataDir: dir, file, log: () => {} });
    expect(await restarted.card("office", fails("Couldn't reach Office."))).toEqual({ state: "stale", report, problem: "Couldn't reach Office." });
    expect(await restarted.card("laptop", fails("Couldn't reach Laptop."))).toEqual({ state: "none", problem: "Couldn't reach Laptop." });
  });
});
