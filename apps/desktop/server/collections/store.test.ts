import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { collectionSlug, fileCollectionStore } from "./store.ts";

function setup() {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-collections-"));
  const errors: unknown[] = [];
  const open = () => fileCollectionStore({ root, onError: (error) => errors.push(error) });
  return { root, errors, open };
}

const monogram = { kind: "monogram", text: "", color: "blue" } as const;

describe("fileCollectionStore", () => {
  it("keeps the list, each collection's file and its SKILL.md, and reads them back", () => {
    const { root, open } = setup();
    const store = open();
    const brand = store.create({ kind: "brand", name: "Ferret Media", parentId: null, orgId: "org_ferret", icon: monogram, rules: "" });
    const series = store.create({
      kind: "series",
      name: "Podcast",
      parentId: brand.id,
      orgId: null,
      icon: { kind: "lucide", name: "mic", color: "violet" },
      rules: "Lower thirds in Inter Bold.\n",
    });

    expect(JSON.parse(fs.readFileSync(path.join(root, "collections.json"), "utf8"))).toEqual({
      version: 1,
      collections: [
        { id: brand.id, folder: "ferret-media" },
        { id: series.id, folder: "podcast" },
      ],
    });
    const dir = path.join(root, "collections", "podcast");
    expect(JSON.parse(fs.readFileSync(path.join(dir, "collection.json"), "utf8"))).toEqual({
      version: 1,
      id: series.id,
      kind: "series",
      name: "Podcast",
      parentId: brand.id,
      orgId: null,
      icon: { kind: "lucide", name: "mic", color: "violet" },
      library: [],
      musicOnlyFromHere: false,
      autonomous: false,
      lessons: [],
      edits: [],
    });
    expect(fs.readFileSync(path.join(dir, "SKILL.md"), "utf8")).toBe(
      '---\nname: podcast\ndescription: "How Podcast videos are edited. Use for every video in this series."\n---\n\nLower thirds in Inter Bold.\n',
    );
    expect(store.rulesFile(series.id)).toBe(path.join(dir, "SKILL.md"));

    // A file from before lessons and working alone loads with neither.
    const file = path.join(dir, "collection.json");
    const { autonomous: _a, lessons: _l, edits: _e, ...older } = JSON.parse(fs.readFileSync(file, "utf8")) as Record<string, unknown>;
    fs.writeFileSync(file, JSON.stringify(older));
    expect(open().load()[1]).toEqual(series);

    // SKILL.md edited in another editor shows on the next start.
    fs.writeFileSync(path.join(dir, "SKILL.md"), "End on the logo.");
    expect(open().load()).toEqual([brand, { ...series, rules: "End on the logo." }]);

    // Front matter someone wrote is theirs, and isn't part of what the app shows.
    const own = "---\r\nname: my-podcast\r\ndescription: Mine.\r\n---\r\n\r\nEnd on the logo.";
    fs.writeFileSync(path.join(dir, "SKILL.md"), own);
    const again = open();
    expect(again.load()[1]!.rules).toBe("End on the logo.");
    again.save({ ...series, rules: "End on black." });
    expect(fs.readFileSync(path.join(dir, "SKILL.md"), "utf8").replaceAll("\r\n", "\n")).toBe("---\nname: my-podcast\ndescription: Mine.\n---\n\nEnd on black.");
    again.save({ ...series, rules: "" });
    expect(fs.readFileSync(path.join(dir, "SKILL.md"), "utf8")).toContain("description: Mine.");

    // What is above a line of dashes is part of the rules.
    const ruled = "---\nNo music.\n\n---\nEnd on the logo.";
    fs.writeFileSync(path.join(dir, "SKILL.md"), ruled);
    expect(open().load()[1]!.rules).toBe(ruled);
  });

  it("takes over the RULES.md of an earlier version, without losing it before SKILL.md is written", () => {
    const { root, open } = setup();
    const made = open().create({ kind: "series", name: "Podcast", parentId: null, orgId: null, icon: monogram, rules: "" });
    const dir = path.join(root, "collections", "podcast");
    fs.rmSync(path.join(dir, "SKILL.md"));
    // With front matter of its own, as rules copied from elsewhere may have: it is kept too.
    const rules = "---\nglobs: video\n---\n\nKeep it short.\n";
    fs.writeFileSync(path.join(dir, "RULES.md"), rules);
    const store = open();
    expect(store.load()).toEqual([{ ...made, rules }]);
    expect(store.rulesFile(made.id)).toBe(path.join(dir, "RULES.md"));
    store.save(store.load()[0]!);
    expect(fs.readdirSync(dir).sort()).toEqual(["SKILL.md", "collection.json"]);
    expect(open().load()).toEqual([{ ...made, rules }]);

    // Written by an earlier version next to a SKILL.md: not read, but not deleted either.
    fs.writeFileSync(path.join(dir, "RULES.md"), "Newer.");
    store.save({ ...made, rules });
    expect(fs.readFileSync(path.join(dir, "RULES.md"), "utf8")).toBe("Newer.");
  });

  it("gives each collection its own folder, and deletes it with the collection", () => {
    const { root, open } = setup();
    const store = open();
    const a = store.create({ kind: "subbrand", name: "Café Ünïcode!", parentId: null, orgId: null, icon: monogram, rules: "" });
    const b = store.create({ kind: "subbrand", name: "cafe unicode", parentId: null, orgId: null, icon: monogram, rules: "x" });
    expect(fs.readdirSync(path.join(root, "collections")).sort()).toEqual(["cafe-unicode", "cafe-unicode-2"]);
    // A skill is named after its folder, and described by its name, also after a rename.
    store.save({ ...b, name: "Studio" });
    expect(fs.readFileSync(path.join(root, "collections", "cafe-unicode-2", "SKILL.md"), "utf8")).toMatch(/^---\nname: cafe-unicode-2\ndescription: "How videos for Studio .*"\n---\n\nx$/);

    const lesson = { id: "l1", rule: "Intro at most 3 seconds.", feedback: "too slow at the start", addedAt: "2026-11-17T10:00:00.000Z", project: "Ep 12" };
    store.save({ ...a, name: "Renamed", rules: "New rules", autonomous: true, lessons: [lesson] });
    expect(open().load()[0]).toMatchObject({ name: "Renamed", rules: "New rules", autonomous: true, lessons: [lesson] });

    // Notes on example videos are files next to the rules, named after the video.
    const file = store.saveExample(a.id, "Reel 12 - Launch!.mp4", "# Reel 12\n");
    expect(file).toBe(path.join(root, "collections", "cafe-unicode", "examples", "reel-12-launch.md"));
    expect(open().examples(a.id)).toEqual([{ name: "reel-12-launch", file }]);
    store.removeExample(a.id, "reel-12-launch");
    expect(store.examples(a.id)).toEqual([]);
    expect(store.saveExample("gone", "x", "y")).toBeNull();

    store.remove(b.id);
    expect(fs.existsSync(path.join(root, "collections", "cafe-unicode-2"))).toBe(false);
    expect(open().load().map((collection) => collection.id)).toEqual([a.id]);
  });

  it("rebuilds a damaged list from the folders, keeping the damaged file", () => {
    const { root, open, errors } = setup();
    const store = open();
    const a = store.create({ kind: "subbrand", name: "Ferret Media", parentId: null, orgId: null, icon: monogram, rules: "Be brief." });
    fs.writeFileSync(path.join(root, "collections.json"), '{"version": 1, "coll');

    expect(open().load()).toEqual([a]);
    expect(fs.readdirSync(root).filter((name) => name.startsWith("collections.json.bad-"))).toHaveLength(1);
    expect(String(errors[0])).toMatch(/rebuilt from the folders/);
    expect(open().load()).toEqual([a]);
  });

  it("leaves a damaged collection.json out, and never writes over it", () => {
    const { root, open, errors } = setup();
    const store = open();
    const a = store.create({ kind: "subbrand", name: "Good", parentId: null, orgId: null, icon: monogram, rules: "" });
    const b = store.create({ kind: "subbrand", name: "Broken", parentId: null, orgId: null, icon: monogram, rules: "" });
    const broken = path.join(root, "collections", "broken", "collection.json");
    fs.writeFileSync(broken, '{"version": 1, "id"');

    const reopened = open();
    expect(reopened.load()).toEqual([a]);
    expect(errors).toHaveLength(1);
    // Saving or making another one with the same name doesn't touch it.
    reopened.save({ ...b, name: "Broken again" });
    const c = reopened.create({ kind: "subbrand", name: "Broken", parentId: null, orgId: null, icon: monogram, rules: "" });
    expect(fs.readFileSync(broken, "utf8")).toBe('{"version": 1, "id"');
    expect(reopened.rulesFile(c.id)).toBe(path.join(root, "collections", "broken-2", "SKILL.md"));
  });

  it("loads a workspace from before brands as a sub-brand in no brand, and keeps its front matter its own", () => {
    const { root, open } = setup();
    const dir = path.join(root, "collections", "studio-north");
    fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(path.join(root, "collections.json"), JSON.stringify({ version: 1, collections: [{ id: "w1", folder: "studio-north" }] }));
    const old = { version: 1, id: "w1", kind: "workspace", name: "Studio North", parentId: null, icon: monogram, library: [], musicOnlyFromHere: false };
    fs.writeFileSync(path.join(dir, "collection.json"), JSON.stringify(old));
    fs.writeFileSync(
      path.join(dir, "SKILL.md"),
      '---\nname: studio-north\ndescription: "How videos for Studio North are edited. Use for every video of this brand."\n---\n\nEnd on the logo.',
    );

    const store = open();
    const [loaded] = store.load();
    expect(loaded).toMatchObject({ id: "w1", kind: "subbrand", parentId: null, orgId: null, rules: "End on the logo." });
    // Its front matter was Linker-Edit's, so it follows a rename.
    store.save({ ...loaded!, name: "North" });
    expect(fs.readFileSync(path.join(dir, "SKILL.md"), "utf8")).toContain('description: "How videos for North are edited.');
    expect(JSON.parse(fs.readFileSync(path.join(dir, "collection.json"), "utf8"))).toMatchObject({ kind: "subbrand", orgId: null });
  });

  it("makes a folder name of any name", () => {
    expect(collectionSlug("  Ferret / Media: 2026  ")).toBe("ferret-media-2026");
    expect(collectionSlug("🎙")).toBe("collection");
    expect(collectionSlug("a".repeat(80))).toHaveLength(50);
  });
});
