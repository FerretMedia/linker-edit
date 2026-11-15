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
  it("keeps the list, each collection's file and its RULES.md, and reads them back", () => {
    const { root, open } = setup();
    const store = open();
    const workspace = store.create({ kind: "workspace", name: "Ferret Media", parentId: null, icon: monogram, rules: "" });
    const series = store.create({
      kind: "series",
      name: "Podcast",
      parentId: workspace.id,
      icon: { kind: "lucide", name: "mic", color: "violet" },
      rules: "Lower thirds in Inter Bold.\n",
    });

    expect(JSON.parse(fs.readFileSync(path.join(root, "collections.json"), "utf8"))).toEqual({
      version: 1,
      collections: [
        { id: workspace.id, folder: "ferret-media" },
        { id: series.id, folder: "podcast" },
      ],
    });
    const dir = path.join(root, "collections", "podcast");
    expect(JSON.parse(fs.readFileSync(path.join(dir, "collection.json"), "utf8"))).toEqual({
      version: 1,
      id: series.id,
      kind: "series",
      name: "Podcast",
      parentId: workspace.id,
      icon: { kind: "lucide", name: "mic", color: "violet" },
      library: [],
      musicOnlyFromHere: false,
    });
    expect(fs.readFileSync(path.join(dir, "RULES.md"), "utf8")).toBe("Lower thirds in Inter Bold.\n");
    expect(store.rulesFile(series.id)).toBe(path.join(dir, "RULES.md"));

    // RULES.md edited in another editor shows on the next start.
    fs.writeFileSync(path.join(dir, "RULES.md"), "End on the logo.");
    expect(open().load()).toEqual([workspace, { ...series, rules: "End on the logo." }]);
  });

  it("gives each collection its own folder, and deletes it with the collection", () => {
    const { root, open } = setup();
    const store = open();
    const a = store.create({ kind: "workspace", name: "Café Ünïcode!", parentId: null, icon: monogram, rules: "" });
    const b = store.create({ kind: "workspace", name: "cafe unicode", parentId: null, icon: monogram, rules: "x" });
    expect(fs.readdirSync(path.join(root, "collections")).sort()).toEqual(["cafe-unicode", "cafe-unicode-2"]);

    store.save({ ...a, name: "Renamed", rules: "New rules" });
    expect(open().load()[0]).toMatchObject({ name: "Renamed", rules: "New rules" });

    store.remove(b.id);
    expect(fs.existsSync(path.join(root, "collections", "cafe-unicode-2"))).toBe(false);
    expect(open().load().map((collection) => collection.id)).toEqual([a.id]);
  });

  it("rebuilds a damaged list from the folders, keeping the damaged file", () => {
    const { root, open, errors } = setup();
    const store = open();
    const a = store.create({ kind: "workspace", name: "Ferret Media", parentId: null, icon: monogram, rules: "Be brief." });
    fs.writeFileSync(path.join(root, "collections.json"), '{"version": 1, "coll');

    expect(open().load()).toEqual([a]);
    expect(fs.readdirSync(root).filter((name) => name.startsWith("collections.json.bad-"))).toHaveLength(1);
    expect(String(errors[0])).toMatch(/rebuilt from the folders/);
    expect(open().load()).toEqual([a]);
  });

  it("leaves a damaged collection.json out, and never writes over it", () => {
    const { root, open, errors } = setup();
    const store = open();
    const a = store.create({ kind: "workspace", name: "Good", parentId: null, icon: monogram, rules: "" });
    const b = store.create({ kind: "workspace", name: "Broken", parentId: null, icon: monogram, rules: "" });
    const broken = path.join(root, "collections", "broken", "collection.json");
    fs.writeFileSync(broken, '{"version": 1, "id"');

    const reopened = open();
    expect(reopened.load()).toEqual([a]);
    expect(errors).toHaveLength(1);
    // Saving or making another one with the same name doesn't touch it.
    reopened.save({ ...b, name: "Broken again" });
    const c = reopened.create({ kind: "workspace", name: "Broken", parentId: null, icon: monogram, rules: "" });
    expect(fs.readFileSync(broken, "utf8")).toBe('{"version": 1, "id"');
    expect(reopened.rulesFile(c.id)).toBe(path.join(root, "collections", "broken-2", "RULES.md"));
  });

  it("makes a folder name of any name", () => {
    expect(collectionSlug("  Ferret / Media: 2026  ")).toBe("ferret-media-2026");
    expect(collectionSlug("🎙")).toBe("collection");
    expect(collectionSlug("a".repeat(80))).toHaveLength(50);
  });
});
