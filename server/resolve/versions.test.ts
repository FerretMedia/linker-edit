// Backups and making an AI timeline the main one, with the real scripts against the stand-in Resolve.
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { runResolveTool } from "./run.ts";
import { hasPython, pythonConnection } from "./testing.ts";
import { applyTimeline, backupProject, KEEP_BACKUPS, readOrigins, recordOrigin } from "./versions.ts";

const FAKE = fs.readFileSync(path.join(import.meta.dirname, "fixtures", "DaVinciResolveScript.py"), "utf8");
const NOW = new Date(2026, 10, 12, 9, 30, 15);

function setup(extra: object = {}) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-versions-"));
  const file = path.join(dir, "state.json");
  const state = {
    project: {
      name: "Doc: final",
      id: "doc-id",
      current: "Edit",
      timelines: [
        { id: "t1", name: "Edit", video: [[]], audio: [] },
        { id: "t2", name: "AI - Edit tight", video: [[]], audio: [] },
        { id: "t3", name: "AI - Other", video: [[]], audio: [] },
      ],
    },
    media: { name: "Master", clips: [], folders: [] },
    ...extra,
  };
  fs.writeFileSync(file, JSON.stringify(state));
  const connection = pythonConnection(FAKE, { FAKE_RESOLVE: file });
  const calls = () =>
    fs.existsSync(`${file}.log`) ? fs.readFileSync(`${file}.log`, "utf8").trim().split("\n").map((line) => JSON.parse(line) as unknown[]) : [];
  const workspace = path.join(dir, "workspace");
  fs.mkdirSync(workspace);
  return { connection, calls, workspace };
}

describe.skipIf(!hasPython)("backups", () => {
  it("saves and exports the open project into backups/, kept out of git", async () => {
    const { connection, calls, workspace } = setup();
    const file = await backupProject(connection, workspace, NOW);
    expect(file).toBe(path.join("backups", "Doc- final 2026-11-12 09-30-15.drp"));
    expect(fs.existsSync(path.join(workspace, file))).toBe(true);
    expect(fs.readFileSync(path.join(workspace, "backups", ".gitignore"), "utf8")).toBe("*\n");
    expect(calls()).toEqual([["SaveProject"], ["ExportProject", "Doc: final", "Doc- final 2026-11-12 09-30-15.drp", false]]);
  });

  it("fails when Resolve can't export", async () => {
    const { connection, workspace } = setup({ export_fails: true });
    await expect(backupProject(connection, workspace, NOW)).rejects.toThrow(/couldn't export "Doc: final"/);
  });

  it(`keeps the newest ${KEEP_BACKUPS}`, async () => {
    const { connection, workspace } = setup();
    const backups = path.join(workspace, "backups");
    fs.mkdirSync(backups);
    for (let i = 0; i < KEEP_BACKUPS + 2; i++) {
      const old = path.join(backups, `old ${i}.drp`);
      fs.writeFileSync(old, "drp");
      fs.utimesSync(old, new Date(2026, 0, 1, 0, i), new Date(2026, 0, 1, 0, i));
    }
    await backupProject(connection, workspace, NOW);
    const left = fs.readdirSync(backups).filter((name) => name.endsWith(".drp"));
    expect(left).toHaveLength(KEEP_BACKUPS);
    expect(left).toContain("Doc- final 2026-11-12 09-30-15.drp");
    expect(left).not.toContain("old 0.drp");
    expect(left).not.toContain("old 2.drp");
    expect(left).toContain("old 3.drp");
  });
});

describe.skipIf(!hasPython)("timeline versions", () => {
  it("remembers what a copy is a version of, through copies of copies", async () => {
    const { connection, workspace } = setup();
    await runResolveTool(connection, "duplicate_timeline", { timeline: "Edit", name: "Tight" }, { dir: workspace });
    expect(Object.values(readOrigins(workspace))).toEqual([{ id: "t1", name: "Edit" }]);
    recordOrigin(workspace, "t4", { id: Object.keys(readOrigins(workspace))[0]!, name: "AI - Tight" });
    expect(readOrigins(workspace).t4).toEqual({ id: "t1", name: "Edit" });
    // A copy of an AI timeline nothing is known about replaces nothing.
    recordOrigin(workspace, "t5", { id: "t3", name: "AI - Other" });
    expect(readOrigins(workspace).t5).toBeUndefined();
  });

  it("remembers what a new timeline replaces, and refuses one that doesn't exist", async () => {
    const { connection, workspace } = setup();
    const made = JSON.parse((await runResolveTool(connection, "create_timeline", { name: "Silences cut", replaces: "Edit" }, { dir: workspace })).text);
    expect(made.replaces).toEqual({ id: "t1", name: "Edit" });
    expect(readOrigins(workspace)[made.id]).toEqual({ id: "t1", name: "Edit" });
    await expect(runResolveTool(connection, "create_timeline", { name: "X", replaces: "Nope" }, { dir: workspace })).rejects.toThrow(/no timeline "Nope"/);
  });

  it("makes an approved AI timeline the main one, keeping the old one renamed", async () => {
    const { connection, calls, workspace } = setup();
    recordOrigin(workspace, "t2", { id: "t1", name: "Edit" });
    recordOrigin(workspace, "t9", { id: "t1", name: "Edit" });
    const applied = await applyTimeline(connection, workspace, "t2", NOW);
    expect(applied).toMatchObject({ main: { id: "t2", name: "Edit" }, previous: { id: "t1", name: "Edit (before AI 2026-11-12 09.30)" }, was: "AI - Edit tight" });
    expect(calls()).toEqual([
      ["SetName", "Edit", "Edit (before AI 2026-11-12 09.30)"],
      ["SetName", "AI - Edit tight", "Edit"],
      ["SetCurrentTimeline", "Edit"],
    ]);
    // The applied one is the main timeline now; other versions of it replace the new main one.
    expect(readOrigins(workspace)).toEqual({ t9: { id: "t2", name: "Edit" } });
  });

  it("only applies an AI timeline it knows the main timeline of", async () => {
    const { connection, calls, workspace } = setup();
    await expect(applyTimeline(connection, workspace, "t2", NOW)).rejects.toThrow(/doesn't know which timeline/);
    recordOrigin(workspace, "t1", { id: "t3", name: "Edit" });
    await expect(applyTimeline(connection, workspace, "t1", NOW)).rejects.toThrow(/isn't an AI timeline/);
    expect(calls()).toEqual([]);
  });

  it("puts the main timeline's name back when the AI timeline can't take it", async () => {
    const { connection, calls, workspace } = setup({ refuse_rename: ["t2"] });
    recordOrigin(workspace, "t2", { id: "t1", name: "Edit" });
    await expect(applyTimeline(connection, workspace, "t2", NOW)).rejects.toThrow(/refused to rename "AI - Edit tight"/);
    expect(calls()).toEqual([
      ["SetName", "Edit", "Edit (before AI 2026-11-12 09.30)"],
      ["SetName", "AI - Edit tight", "Edit"],
      ["SetName", "Edit (before AI 2026-11-12 09.30)", "Edit"],
    ]);
    // Nothing changed, so it can be tried again.
    expect(readOrigins(workspace)).toEqual({ t2: { id: "t1", name: "Edit" } });
  });
});
