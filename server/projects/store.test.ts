import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { fileProjectStore, type SavedThread } from "./store.ts";

function setup() {
  const home = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-projects-"));
  const root = path.join(home, "projects");
  const legacyDir = path.join(home, "workspace");
  const errors: unknown[] = [];
  const open = () => fileProjectStore({ root, legacyDir, onError: (error) => errors.push(error) });
  return { home, root, legacyDir, errors, open };
}

const thread = (fields: Partial<SavedThread> = {}): SavedThread => ({
  id: "t1",
  title: "Cut the silences",
  provider: "codex",
  resumeId: "thread-1",
  items: [{ id: "1", kind: "user", text: "Cut the silences" }],
  context: null,
  createdAt: "2026-11-09T10:00:00.000Z",
  updatedAt: "2026-11-09T10:00:00.000Z",
  archivedAt: null,
  ...fields,
});

describe("fileProjectStore", () => {
  it("makes the old workspace the first project, in place, with its chat as the first thread", () => {
    const { legacyDir, root, open } = setup();
    fs.mkdirSync(path.join(legacyDir, ".linker-edit"), { recursive: true });
    const chat = { provider: "claude", resumeId: "abc", items: [{ id: "1", kind: "user", text: "Rough cut\nof the interview" }], context: null };
    fs.writeFileSync(path.join(legacyDir, ".linker-edit", "chat.json"), JSON.stringify({ version: 1, chat }));

    const [first, ...rest] = open().load();
    expect(rest).toEqual([]);
    expect(first!.project).toMatchObject({ dir: legacyDir, name: null, link: null, local: {} });
    expect(first!.threads).toMatchObject([{ title: "Rough cut", provider: "claude", resumeId: "abc", archivedAt: null }]);
    expect(fs.existsSync(path.join(legacyDir, ".linker-edit", "chat.json"))).toBe(false);
    expect(fs.readFileSync(path.join(legacyDir, ".linker-edit", ".gitignore"), "utf8")).toBe("*\n");

    // The next start reads the registry and the migrated thread, and doesn't register the folder again.
    expect(open().load()).toMatchObject([{ project: { id: first!.project.id, dir: legacyDir }, threads: [{ resumeId: "abc" }] }]);
    expect(fs.existsSync(path.join(root, "projects.json"))).toBe(true);
  });

  it("creates a folder per project, keeping the link in linker-edit.json and this machine's side out of it", () => {
    const { root, open } = setup();
    const store = open();
    const link = { kind: "cloud" as const, name: 'Client: "X"', projectId: null };
    const a = store.create({ name: null, link, local: { mediaPath: "D:\\Cloud", syncMode: "proxy_only" } });
    const b = store.create({ name: null, link, local: {} });
    expect([a.dir, b.dir]).toEqual([path.join(root, "Client X"), path.join(root, "Client X 2")]);
    expect(JSON.parse(fs.readFileSync(path.join(a.dir, "linker-edit.json"), "utf8"))).toEqual({ version: 1, name: null, resolve: link });

    store.saveThread(a, thread());
    store.saveThread(a, thread({ id: "t2", archivedAt: "2026-11-09T11:00:00.000Z" }));
    store.deleteThread(a, "t2");
    store.removeProject(b.id);

    const loaded = open().load();
    expect(loaded.map(({ project }) => project.dir)).toEqual([expect.stringContaining("workspace"), a.dir]);
    expect(loaded[1]).toEqual({ project: a, threads: [thread()] });
    expect(fs.existsSync(b.dir)).toBe(true);
  });

  it("adopts a cloned project folder with its link, and only once", () => {
    const { home, open } = setup();
    const store = open();
    const clone = path.join(home, "clone");
    fs.mkdirSync(clone);
    expect(() => store.adopt(clone)).toThrow("has no linker-edit.json");
    const link = { kind: "cloud", name: "Client X", projectId: "cx" };
    fs.writeFileSync(path.join(clone, "linker-edit.json"), JSON.stringify({ version: 1, name: "Client X edit", resolve: link }));

    expect(store.adopt(clone)).toMatchObject({ project: { dir: clone, name: "Client X edit", link, local: {} }, threads: [] });
    expect(() => store.adopt(clone)).toThrow("already a project");
  });

  it("rebuilds a damaged project list from the project folders, keeping the damaged file", () => {
    const { home, root, open, errors } = setup();
    const store = open();
    const a = store.create({ name: "Doc", link: null, local: {} });
    store.saveThread(a, thread());
    fs.writeFileSync(path.join(root, "projects.json"), '{"version": 1, "proj');

    const loaded = open().load();
    expect(loaded.map(({ project }) => project.dir)).toEqual([path.join(home, "workspace"), a.dir]);
    expect(loaded[1]!.threads).toEqual([thread()]);
    expect(fs.readdirSync(root).filter((name) => name.startsWith("projects.json.bad-"))).toHaveLength(1);
    expect(String(errors[0])).toMatch(/rebuilt from the folders/);
    // The rebuilt list is a good one: the next start reads it as it is.
    expect(open().load().map(({ project }) => project.id)).toEqual(loaded.map(({ project }) => project.id));
  });

  it("skips a damaged thread file and says so", () => {
    const { open, errors } = setup();
    const store = open();
    const project = store.create({ name: "Doc", link: null, local: {} });
    store.saveThread(project, thread());
    fs.writeFileSync(path.join(project.dir, ".linker-edit", "threads", "t2.json"), '{"version": 1, "thread": {');
    expect(open().load()[1]!.threads).toEqual([thread()]);
    expect(errors).toHaveLength(1);
  });
});
