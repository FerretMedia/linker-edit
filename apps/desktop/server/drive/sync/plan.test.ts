import { describe, expect, it } from "vitest";
import type { DriveChange, DriveFile } from "../client.ts";
import { applyChanges, planSync, remotePaths, type Action, type BaseFile, type LocalFile, type RemoteTree, type RemoteView } from "./plan.ts";

const here = (md5: string, mtimeMs = 1): LocalFile => ({ md5, size: md5.length, mtimeMs });
const agreed = (driveId: string, md5: string, mtimeMs = 1): BaseFile => ({ driveId, md5, size: md5.length, mtimeMs });
const onDrive = (driveId: string, md5: string) => ({ driveId, md5, size: md5.length });

interface Row {
  base?: Record<string, BaseFile>;
  local?: Record<string, LocalFile>;
  remote?: RemoteView["files"];
  actions: Action[];
}

const CASES: Record<string, Row> = {
  "new on this computer": { local: { "a.mov": here("aa") }, actions: [{ kind: "upload", path: "a.mov", driveId: null }] },
  "new on Drive": { remote: { "a.mov": onDrive("d1", "aa") }, actions: [{ kind: "download", path: "a.mov", driveId: "d1" }] },
  "new on both, the same bytes": {
    local: { "a.mov": here("aa", 5) },
    remote: { "a.mov": onDrive("d1", "aa") },
    actions: [{ kind: "agree", path: "a.mov", file: agreed("d1", "aa", 5) }],
  },
  "new on both, different bytes": {
    local: { "notes.txt": here("mine") },
    remote: { "notes.txt": onDrive("d1", "theirs") },
    actions: [
      { kind: "setAside", path: "notes.txt", copy: "notes (from Studio).txt" },
      { kind: "download", path: "notes.txt", driveId: "d1" },
      { kind: "upload", path: "notes (from Studio).txt", driveId: null },
    ],
  },
  unchanged: { base: { "a.mov": agreed("d1", "aa") }, local: { "a.mov": here("aa") }, remote: { "a.mov": onDrive("d1", "aa") }, actions: [] },
  "touched here, same bytes": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "a.mov": here("aa", 9) },
    remote: { "a.mov": onDrive("d1", "aa") },
    actions: [{ kind: "agree", path: "a.mov", file: agreed("d1", "aa", 9) }],
  },
  "changed here": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "a.mov": here("bb", 2) },
    remote: { "a.mov": onDrive("d1", "aa") },
    actions: [{ kind: "upload", path: "a.mov", driveId: "d1" }],
  },
  "changed on Drive": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "a.mov": here("aa") },
    remote: { "a.mov": onDrive("d1", "cc") },
    actions: [{ kind: "download", path: "a.mov", driveId: "d1" }],
  },
  "replaced on Drive by another file with the same bytes": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "a.mov": here("aa") },
    remote: { "a.mov": onDrive("d2", "aa") },
    actions: [{ kind: "agree", path: "a.mov", file: agreed("d2", "aa") }],
  },
  "changed on both, the same way": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "a.mov": here("bb", 3) },
    remote: { "a.mov": onDrive("d1", "bb") },
    actions: [{ kind: "agree", path: "a.mov", file: agreed("d1", "bb", 3) }],
  },
  "changed on both, differently": {
    base: { "cut/notes.txt": agreed("d1", "old") },
    local: { "cut/notes.txt": here("mine", 2) },
    remote: { "cut/notes.txt": onDrive("d1", "theirs") },
    actions: [
      { kind: "setAside", path: "cut/notes.txt", copy: "cut/notes (from Studio).txt" },
      { kind: "download", path: "cut/notes.txt", driveId: "d1" },
      { kind: "upload", path: "cut/notes (from Studio).txt", driveId: null },
    ],
  },
  "changed on both, and the copy's name is taken": {
    base: { "notes.txt": agreed("d1", "old") },
    local: { "notes.txt": here("mine", 2), "notes (from Studio).txt": here("earlier") },
    remote: { "notes.txt": onDrive("d1", "theirs"), "notes (from Studio).txt": onDrive("d2", "earlier") },
    actions: [
      { kind: "setAside", path: "notes.txt", copy: "notes (from Studio 2).txt" },
      { kind: "agree", path: "notes (from Studio).txt", file: agreed("d2", "earlier") },
      { kind: "download", path: "notes.txt", driveId: "d1" },
      { kind: "upload", path: "notes (from Studio 2).txt", driveId: null },
    ],
  },
  "deleted here": {
    base: { "a.mov": agreed("d1", "aa") },
    remote: { "a.mov": onDrive("d1", "aa") },
    actions: [{ kind: "deleteRemote", path: "a.mov", driveId: "d1" }],
  },
  "deleted on Drive": { base: { "a.mov": agreed("d1", "aa") }, local: { "a.mov": here("aa") }, actions: [{ kind: "deleteLocal", path: "a.mov" }] },
  "deleted on both": { base: { "a.mov": agreed("d1", "aa") }, actions: [{ kind: "forget", path: "a.mov" }] },
  "deleted here, changed on Drive": {
    base: { "a.mov": agreed("d1", "aa") },
    remote: { "a.mov": onDrive("d1", "cc") },
    actions: [{ kind: "download", path: "a.mov", driveId: "d1" }],
  },
  "changed here, deleted on Drive": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "a.mov": here("bb", 2) },
    actions: [{ kind: "upload", path: "a.mov", driveId: null }],
  },
  "renamed here": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "intro.mov": here("aa", 4) },
    remote: { "a.mov": onDrive("d1", "aa") },
    actions: [{ kind: "moveRemote", from: "a.mov", to: "intro.mov", driveId: "d1" }],
  },
  "moved into a folder on Drive": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "a.mov": here("aa") },
    remote: { "media/a.mov": onDrive("d1", "aa") },
    actions: [{ kind: "moveLocal", from: "a.mov", to: "media/a.mov" }],
  },
  "renamed and changed on Drive": {
    base: { "a.mov": agreed("d1", "aa") },
    local: { "a.mov": here("aa") },
    remote: { "intro.mov": onDrive("d1", "cc") },
    actions: [
      { kind: "deleteLocal", path: "a.mov" },
      { kind: "download", path: "intro.mov", driveId: "d1" },
    ],
  },
  "this computer's own files": {
    local: {
      ".git/HEAD": here("h"),
      ".git": here("g"),
      ".linker-edit/threads/t1.json": here("t"),
      ".linker-edit/local.json": here("l"),
      ".linker-edit/drive-manifest.json": here("m"),
      ".linker-edit/drive-downloads/d9-aa.part": here("p"),
      ".gitignore": here("i"),
    },
    remote: { ".git/config": onDrive("d1", "c"), ".linker-edit/threads/t2.json": onDrive("d2", "t") },
    actions: [{ kind: "upload", path: ".gitignore", driveId: null }],
  },
};

describe("planSync", () => {
  it.each(Object.entries(CASES))("%s", (_name, row) => {
    expect(planSync({ base: row.base ?? {}, local: row.local ?? {}, remote: row.remote ?? {}, computer: "Studio" })).toEqual(row.actions);
  });
});

const ROOT = "project";
const drive = (id: string, name: string, parent: string, extra: Partial<DriveFile> = {}): DriveChange => ({
  fileId: id,
  removed: false,
  file: { id, name, parents: [parent], mimeType: "video/quicktime", md5: `md5-${id}`, size: 10, modifiedTime: "2026-11-26T10:00:00.000Z", trashed: false, ...extra },
});
const folder = (id: string, name: string, parent: string) => drive(id, name, parent, { mimeType: "application/vnd.google-apps.folder", md5: null, size: null });

describe("Drive's changes feed", () => {
  it("keeps what is inside the project's folder, whatever order the feed lists it in", () => {
    const tree = applyChanges(
      { folders: {}, files: {} },
      [
        drive("clip", "clip.mov", "media"),
        folder("media", "Media", ROOT),
        drive("notes", "notes.txt", ROOT),
        drive("elsewhere", "other.mov", "another-project"),
        folder("another-project", "Other", "root"),
        drive("doc", "Script", ROOT, { mimeType: "application/vnd.google-apps.document", md5: null, size: null }),
      ],
      ROOT,
    );
    expect(remotePaths(tree, ROOT)).toEqual({
      folders: { "": ROOT, Media: "media" },
      files: { "Media/clip.mov": { driveId: "clip", md5: "md5-clip", size: 10 }, "notes.txt": { driveId: "notes", md5: "md5-notes", size: 10 } },
    });
  });

  it("follows a renamed folder, and drops what was trashed, deleted or moved out", () => {
    const before: RemoteTree = {
      folders: { media: { name: "Media", parent: ROOT } },
      files: {
        clip: { name: "clip.mov", parent: "media", md5: "c", size: 1 },
        old: { name: "old.mov", parent: ROOT, md5: "o", size: 1 },
        gone: { name: "gone.mov", parent: ROOT, md5: "g", size: 1 },
        away: { name: "away.mov", parent: ROOT, md5: "a", size: 1 },
      },
    };
    const after = applyChanges(
      before,
      [folder("media", "Footage", ROOT), drive("old", "old.mov", ROOT, { trashed: true }), { fileId: "gone", removed: true }, drive("away", "away.mov", "root")],
      ROOT,
    );
    expect(Object.keys(remotePaths(after, ROOT).files)).toEqual(["Footage/clip.mov"]);
  });

  it("gives a name Drive has twice in one folder a number, the same on every computer", () => {
    const tree: RemoteTree = {
      folders: {},
      files: { b: { name: "notes.txt", parent: ROOT, md5: "2", size: 1 }, a: { name: "notes.txt", parent: ROOT, md5: "1", size: 1 } },
    };
    expect(remotePaths(tree, ROOT).files).toEqual({ "notes.txt": { driveId: "a", md5: "1", size: 1 }, "notes (2).txt": { driveId: "b", md5: "2", size: 1 } });
  });
});
