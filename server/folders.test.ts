import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { browseFolders, expandHome, listRoots } from "./folders.ts";

const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-folders-"));
for (const dir of ["Promo", "b-roll", ".git", "Clip 10", "Clip 9"]) fs.mkdirSync(path.join(root, dir));
fs.writeFileSync(path.join(root, "Promo", "linker-edit.json"), "{}");
fs.writeFileSync(path.join(root, "notes.txt"), "");
fs.writeFileSync(path.join(root, "linker-edit.json"), "{}");
afterAll(() => fs.rmSync(root, { recursive: true, force: true }));

describe("browseFolders", () => {
  it("lists only folders, sorted the way people count, marking project folders", async () => {
    const listing = await browseFolders(root);
    expect(listing.dir).toBe(root + path.sep);
    expect(listing.isProject).toBe(true);
    // Dot-folders are sent too: the picker shows them once a dot is typed.
    expect(listing.entries).toEqual([
      { name: ".git", isProject: false },
      { name: "b-roll", isProject: false },
      { name: "Clip 9", isProject: false },
      { name: "Clip 10", isProject: false },
      { name: "Promo", isProject: true },
    ]);
    expect(listing.roots.length).toBeGreaterThan(0);
  });

  it("starts in the home folder for an empty path and expands ~", async () => {
    expect((await browseFolders("", root)).dir).toBe(root + path.sep);
    expect((await browseFolders(`~${path.sep}Promo`, root)).dir).toBe(path.join(root, "Promo") + path.sep);
    expect((await browseFolders("~", root)).home).toBe(root + path.sep);
  });

  it("refuses relative paths and says when a folder is missing or a file", async () => {
    await expect(browseFolders("Promo", root)).rejects.toThrow(/Start the path/);
    await expect(browseFolders(path.join(root, "missing"))).rejects.toThrow(/There's no folder/);
    await expect(browseFolders(path.join(root, "notes.txt"))).rejects.toThrow(/is a file/);
  });
});

describe("expandHome", () => {
  it("expands only a leading ~", () => {
    expect(expandHome("~", "/home/me")).toBe("/home/me");
    expect(expandHome(" ~/Videos ", "/home/me")).toBe(path.join("/home/me", "Videos"));
    expect(expandHome("D:/~media", "/home/me")).toBe("D:/~media");
  });
});

describe("listRoots", () => {
  it("includes the drive or root this folder is on", async () => {
    const own = path.parse(root).root;
    expect((await listRoots()).map((entry) => entry.toUpperCase())).toContain(own.toUpperCase());
  });
});
