import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { PARENT_KINDS, type AppState, type ClientMessage, type CollectionKind, type LinkerOrg, type ServerMessage } from "../../shared/protocol.ts";
import { applyServerMessage } from "../../src/lib/appState.ts";
import { createApp } from "../app.ts";
import type { ProjectCollection, SavedProject } from "../projects/store.ts";
import { memoryProjectStore } from "../projects/testing.ts";
import { EDITING_RULES, PROJECT_RULES } from "../rules.ts";
import { createLinkerSkills } from "../skills/linkerSkills.ts";
import type { SkillRoot } from "../skills/skills.ts";
import { serveFakeLinkerSkills } from "../skills/testing.ts";
import { collectionChain, createCollections, parentRefusal } from "./collections.ts";
import { fileCollectionStore, type SavedCollection } from "./store.ts";
import { linkerWithOrgs, memoryCollectionStore } from "./testing.ts";

type App = ReturnType<typeof createApp>;

const org = (id: string, name: string): LinkerOrg => ({ id, name, imageUrl: `https://img.example/${id}.png` });
const FERRET = org("org_ferret", "Ferret Media");

function setup(projects: SavedProject[] = [], skills: SkillRoot[] = [], initial: SavedCollection[] = []) {
  const store = memoryCollectionStore(initial);
  const saved = new Map(projects.map((project) => [project.id, project]));
  const commits: string[] = [];
  let orgs: LinkerOrg[] | null = [FERRET];
  let changes = 0;
  const collections = createCollections({
    store,
    skills,
    projects: () => [...saved.values()],
    orgs: () => orgs,
    setCollection(projectId, collection: ProjectCollection | null, commitMessage) {
      saved.set(projectId, { ...saved.get(projectId)!, collection });
      commits.push(commitMessage);
    },
    removeProject: (projectId) => saved.delete(projectId),
    changed: () => (changes += 1),
  });
  collections.syncBrands();
  const send = (message: ClientMessage) => collections.handle(message);
  const byName = (name: string) => collections.summaries().find((collection) => collection.name === name)!;
  /** Linker answers with these orgs from now on; null is signed out. */
  const setOrgs = (next: LinkerOrg[] | null) => {
    orgs = next;
    collections.syncBrands();
  };
  return { collections, store, saved, commits, send, byName, setOrgs, changes: () => changes };
}

const project = (id: string, dir = `/projects/${id}`): SavedProject => ({ id, dir, name: id, link: null, local: {}, icon: null, pinned: false, archived: false });

const collection = (id: string, kind: CollectionKind, parentId: string | null): SavedCollection => ({
  id,
  kind,
  name: id,
  parentId,
  orgId: kind === "brand" ? `org_${id}` : null,
  icon: { kind: "monogram", text: "", color: "blue" },
  rules: "",
  library: [],
  musicOnlyFromHere: false,
  autonomous: false,
  lessons: [],
  edits: [],
});

describe("where a brand, sub-brand or series can go", () => {
  const kinds = Object.keys(PARENT_KINDS) as CollectionKind[];
  const pairs = kinds.flatMap((kind) => [null, ...kinds].map((parent) => [kind, parent] as const));
  it.each(pairs)("a %s in %s", (kind, parent) => {
    const allowed = { brand: [null], subbrand: ["brand"], series: ["brand", "subbrand"] }[kind] as (CollectionKind | null)[];
    expect(parentRefusal(kind, parent) === null).toBe(allowed.includes(parent));
  });

  it("says what is allowed when it isn't", () => {
    expect(parentRefusal("series", "series")).toBe("A series can only go in a brand or sub-brand.");
    expect(parentRefusal("subbrand", "subbrand")).toBe("A sub-brand can only go in a brand.");
    expect(parentRefusal("subbrand", null)).toBe("A sub-brand can only go in a brand.");
    expect(parentRefusal("brand", "brand")).toBe("A brand can't go inside anything.");
  });
});

describe("collectionChain", () => {
  const all = new Map(
    [
      collection("ferret", "brand", null),
      collection("shorts", "subbrand", "ferret"),
      collection("weekly", "series", "shorts"),
      collection("podcast", "series", "ferret"),
    ].map((entry) => [entry.id, entry]),
  );
  const inside = (id: string) => ({ ...project(id), collection: { id, name: id } });
  it.each([
    ["brand, project", "ferret", ["ferret"]],
    ["brand, series, project", "podcast", ["ferret", "podcast"]],
    ["brand, sub-brand, project", "shorts", ["ferret", "shorts"]],
    ["brand, sub-brand, series, project", "weekly", ["ferret", "shorts", "weekly"]],
  ])("%s, root first", (_shape, id, chain) => {
    expect(collectionChain(all, inside(id)).map((entry) => entry.id)).toEqual(chain);
  });

  it("is empty for a project in none, or in one that's gone", () => {
    expect(collectionChain(all, project("loose"))).toEqual([]);
    expect(collectionChain(all, inside("gone"))).toEqual([]);
  });
});

describe("brands, sub-brands and series", () => {
  it("makes a brand for each Linker org, greys it when the person leaves, and follows a rename", () => {
    const { byName, setOrgs, send, collections, store } = setup();
    expect(byName("Ferret Media")).toMatchObject({
      kind: "brand",
      parentId: null,
      org: { id: "org_ferret", imageUrl: "https://img.example/org_ferret.png", member: true },
    });

    setOrgs([FERRET, org("org_north", "Studio North")]);
    expect(collections.summaries().map((entry) => [entry.name, entry.org?.member])).toEqual([
      ["Ferret Media", true],
      ["Studio North", true],
    ]);

    // Left on Linker: it stays on disk, greyed, and takes nothing new.
    const north = byName("Studio North").id;
    setOrgs([FERRET]);
    expect(byName("Studio North").org).toEqual({ id: "org_north", imageUrl: null, member: false });
    expect(store.collections.has(north)).toBe(true);
    expect(send({ type: "create_collection", kind: "subbrand", name: "Shorts", parentId: north })).toBe(
      'You\'re not in "Studio North" on Linker any more, so it takes nothing new.',
    );
    setOrgs(null);
    expect(byName("Ferret Media").org?.member).toBe(false);
    expect(send({ type: "create_collection", kind: "series", name: "Weekly", parentId: byName("Ferret Media").id })).toMatch(/^Sign in to Linker/);

    // Linker's name wins, and only one brand per org.
    setOrgs([{ ...FERRET, name: "Ferret Studios" }]);
    expect(collections.summaries().filter((entry) => entry.kind === "brand").map((entry) => entry.name)).toEqual(["Ferret Studios", "Studio North"]);
    expect(send({ type: "update_collection", id: byName("Ferret Studios").id, name: "Mine" })).toMatch(/comes from Linker/);
    expect(send({ type: "create_collection", kind: "brand", name: "Mine" })).toMatch(/are your Linker orgs/);
    expect(send({ type: "delete_collection", id: byName("Ferret Studios").id })).toMatch(/is a Linker org, so it stays/);
  });

  it("builds all four shapes, and moves sub-brands and series only where PARENT_KINDS allows", () => {
    const { send, byName, collections } = setup();
    const brand = byName("Ferret Media").id;
    expect(send({ type: "create_collection", kind: "subbrand", name: "  Shorts  ", parentId: brand })).toBeNull();
    expect(send({ type: "create_collection", kind: "series", name: "Weekly", parentId: byName("Shorts").id })).toBeNull();
    expect(send({ type: "create_collection", kind: "series", name: "Podcast", parentId: brand })).toBeNull();
    expect(byName("Shorts")).toMatchObject({ kind: "subbrand", parentId: brand, org: null });

    expect(send({ type: "create_collection", kind: "series", name: "Clips", parentId: byName("Weekly").id })).toBe("A series can only go in a brand or sub-brand.");
    expect(send({ type: "create_collection", kind: "subbrand", name: "Loose" })).toBe("A sub-brand can only go in a brand.");
    expect(send({ type: "create_collection", kind: "series", name: "Gone", parentId: "gone" })).toMatch(/is gone/);
    expect(send({ type: "create_collection", kind: "series", name: "   ", parentId: brand })).toMatch(/name/);

    // A series moves between its brand and a sub-brand; a sub-brand never goes in a sub-brand or nowhere.
    expect(send({ type: "update_collection", id: byName("Podcast").id, parentId: byName("Shorts").id })).toBeNull();
    expect(byName("Podcast").parentId).toBe(byName("Shorts").id);
    expect(send({ type: "update_collection", id: byName("Shorts").id, parentId: byName("Podcast").id })).toBe("A sub-brand can only go in a brand.");
    expect(send({ type: "update_collection", id: byName("Shorts").id, parentId: null })).toBe("A sub-brand can only go in a brand.");
    // Brands first, then sub-brands, then series.
    expect(collections.summaries().map((entry) => entry.name)).toEqual(["Ferret Media", "Shorts", "Weekly", "Podcast"]);
  });

  it("loads one from before brands under no brand, and lets it move into one", () => {
    const old = { ...collection("studio", "subbrand", null), name: "Studio" };
    const loose = { ...collection("reels", "series", null), name: "Reels" };
    const lost = { ...collection("lost", "series", "nowhere"), name: "Lost" };
    const { byName, send } = setup([], [], [old, loose, lost]);
    expect(byName("Studio").parentId).toBeNull();
    expect(byName("Lost").parentId).toBeNull();
    // A series can still go in it, as in a sub-brand.
    expect(send({ type: "update_collection", id: "reels", parentId: "studio" })).toBeNull();
    expect(send({ type: "update_collection", id: "studio", parentId: byName("Ferret Media").id })).toBeNull();
    expect(byName("Studio").parentId).toBe(byName("Ferret Media").id);
    expect(byName("Reels").parentId).toBe("studio");
  });

  it("refuses to delete one that still has projects, sub-brands or series in it", () => {
    const { send, byName, store, collections } = setup([project("ep12"), project("ep11")]);
    send({ type: "create_collection", kind: "subbrand", name: "Shorts", parentId: byName("Ferret Media").id });
    const shorts = byName("Shorts");
    send({ type: "create_collection", kind: "series", name: "Weekly", parentId: shorts.id });
    const weekly = byName("Weekly");
    send({ type: "move_project", projectId: "ep12", collectionId: weekly.id });
    send({ type: "move_project", projectId: "ep11", collectionId: weekly.id });

    expect(send({ type: "delete_collection", id: shorts.id })).toBe('Sub-brand "Shorts" still has 1 series in it. Move it out first.');
    expect(send({ type: "delete_collection", id: weekly.id })).toBe('Series "Weekly" still has 2 projects in it. Move them out first.');
    expect(store.collections.size).toBe(3);

    send({ type: "move_project", projectId: "ep12", collectionId: null });
    send({ type: "move_project", projectId: "ep11", collectionId: shorts.id });
    expect(send({ type: "delete_collection", id: weekly.id })).toBeNull();
    expect(send({ type: "delete_collection", id: shorts.id })).toMatch(/still has 1 project in it/);
    expect(collections.summaries().map((entry) => entry.name)).toEqual(["Ferret Media", "Shorts"]);
  });

  it("puts projects in a collection and keeps its name in them when it's renamed", () => {
    const { send, byName, saved, commits, changes, setOrgs } = setup([project("ep12")]);
    send({ type: "create_collection", kind: "series", name: "Podcast", parentId: byName("Ferret Media").id });
    const series = byName("Podcast");
    send({ type: "move_project", projectId: "ep12", collectionId: series.id });
    expect(saved.get("ep12")!.collection).toEqual({ id: series.id, name: "Podcast" });
    expect(byName("Podcast").projectIds).toEqual(["ep12"]);

    send({ type: "update_collection", id: series.id, name: "The Podcast", rules: "Keep it short." });
    expect(saved.get("ep12")!.collection).toEqual({ id: series.id, name: "The Podcast" });
    expect(byName("The Podcast").rules).toBe("Keep it short.");
    expect(send({ type: "move_project", projectId: "ep12", collectionId: "gone" })).toMatch(/is gone/);
    expect(commits).toEqual(['Put the project in Series "Podcast"', 'Series "The Podcast" renamed']);
    // One more for the brand Linker listed.
    expect(changes()).toBe(4);

    // A brand renamed in Linker is renamed in its projects too; one the person left takes no project.
    send({ type: "move_project", projectId: "ep12", collectionId: byName("Ferret Media").id });
    setOrgs([{ ...FERRET, name: "Ferret Studios" }]);
    expect(saved.get("ep12")!.collection).toEqual({ id: byName("Ferret Studios").id, name: "Ferret Studios" });
    setOrgs([]);
    expect(send({ type: "move_project", projectId: "ep12", collectionId: series.id })).toMatch(/not in "Ferret Studios" on Linker any more/);
  });

  it("refuses an icon that isn't one", () => {
    const { send, byName } = setup();
    send({ type: "create_collection", kind: "subbrand", name: "W", parentId: byName("Ferret Media").id, icon: { kind: "emoji", emoji: "🎙", color: "pink" } });
    const id = byName("W").id;
    send({ type: "update_collection", id, icon: { kind: "lucide", name: "<script>", color: "blue" } });
    send({ type: "update_collection", id, icon: { kind: "monogram", text: "AB", color: "nope" } as never });
    expect(byName("W").icon).toEqual({ kind: "emoji", emoji: "🎙", color: "pink" });
    send({ type: "update_collection", id, icon: { kind: "lucide", name: "mic", color: "teal" } });
    expect(byName("W").icon).toEqual({ kind: "lucide", name: "mic", color: "teal" });
  });

  it("stacks the agent's rules: editing rules, brand, sub-brand, series, then the project's own, most specific last", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-rules-"));
    fs.writeFileSync(path.join(dir, "PROJECT.md"), "Ep 12 is the anniversary episode.\n");
    // A project from before PROJECT.md keeps the notes it had.
    fs.writeFileSync(path.join(dir, "RULES.md"), "Guest asked to cut the salary talk.\n");
    const { send, byName, collections, saved } = setup([project("ep12", dir), project("ep11")]);
    const brand = byName("Ferret Media").id;
    send({ type: "update_collection", id: brand, rules: "End on the logo." });
    send({ type: "create_collection", kind: "subbrand", name: "Shorts", parentId: brand });
    send({ type: "update_collection", id: byName("Shorts").id, rules: "Vertical, 9:16." });
    send({ type: "create_collection", kind: "series", name: "Weekly", parentId: byName("Shorts").id });
    send({ type: "update_collection", id: byName("Weekly").id, rules: "Lower thirds in Inter Bold.", autonomous: true });
    send({ type: "move_project", projectId: "ep12", collectionId: byName("Weekly").id });
    send({ type: "move_project", projectId: "ep11", collectionId: byName("Shorts").id });

    const rules = collections.rulesFor(saved.get("ep12")!);
    expect(rules.startsWith(EDITING_RULES)).toBe(true);
    const order = [
      "# The user's rules",
      `## Brand "Ferret Media" (/collections/${brand}/SKILL.md)\n\nEnd on the logo.`,
      `## Sub-brand "Shorts" (/collections/${byName("Shorts").id}/SKILL.md)\n\nVertical, 9:16.`,
      `## Series "Weekly" (/collections/${byName("Weekly").id}/SKILL.md)\n\nLower thirds in Inter Bold.`,
      "## This project (PROJECT.md in the working folder)\n\nEp 12 is the anniversary episode.\n\nGuest asked to cut the salary talk.",
    ].map((part) => rules.indexOf(part));
    expect(order.every((index, i) => index > 0 && (i === 0 || index > order[i - 1]!))).toBe(true);
    // Working alone follows the nearest one.
    expect(collections.autonomous(saved.get("ep12")!)).toBe(true);
    expect(collections.autonomous(saved.get("ep11")!)).toBe(false);

    // Without notes of its own, a project in the sub-brand gets the brand's and the sub-brand's rules only.
    const other = collections.rulesFor(saved.get("ep11")!);
    expect(other).toContain("End on the logo.");
    expect(other).toContain("Vertical, 9:16.");
    expect(other).not.toContain("Lower thirds in Inter Bold.");
    expect(other).not.toContain("## This project (");

    // A project in no collection, with no file yet, gets the editing rules and how to keep its file.
    expect(collections.rulesFor(project("loose"))).toBe(`${EDITING_RULES}\n\n${PROJECT_RULES}`);
  });

  it("lists the skills last, and looks for them from Linker-Edit's to the series'", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-skills-"));
    fs.mkdirSync(path.join(dir, "sound"));
    fs.writeFileSync(path.join(dir, "sound", "SKILL.md"), "---\ndescription: Music and levels.\n---\n\nUnder speech.");
    const shipped = { label: "Linker-Edit", dir };
    const { send, byName, collections, saved } = setup([project("ep12"), project("loose")], [shipped]);
    send({ type: "create_collection", kind: "subbrand", name: "Shorts", parentId: byName("Ferret Media").id });
    send({ type: "create_collection", kind: "series", name: "Weekly", parentId: byName("Shorts").id });
    send({ type: "move_project", projectId: "ep12", collectionId: byName("Weekly").id });
    expect(collections.skillRoots(saved.get("loose")!)).toEqual([shipped]);
    expect(collections.skillRoots(saved.get("ep12")!)).toEqual([
      shipped,
      { label: 'Brand "Ferret Media"', dir: path.join(`/collections/${byName("Ferret Media").id}`, "skills") },
      { label: 'Sub-brand "Shorts"', dir: path.join(`/collections/${byName("Shorts").id}`, "skills") },
      { label: 'Series "Weekly"', dir: path.join(`/collections/${byName("Weekly").id}`, "skills") },
    ]);
    expect(collections.rulesFor(saved.get("loose")!)).toMatch(/\n\n# Skills\n\n.+\n\n- sound: Music and levels\. \(Linker-Edit\)$/);
  });
});

describe("brands in the app", () => {
  it("sends collections to every client, puts a new project in one, and gives both agents the project's rules at each turn", async () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-file-"));
    const projects = memoryProjectStore([{ project: project("ep12", dir) }, { project: project("ep11") }]);
    const prepared = new Map<string, string>();
    const rules: string[] = [];
    const app = createApp({
      version: "0.0.0",
      store: projects,
      collections: memoryCollectionStore(),
      linker: linkerWithOrgs([FERRET]),
      prepareFolder: (dir, text) => prepared.set(dir, text),
      createSession: (_provider, _mode, _dir, _resumeId, projectRules) => ({
        send: async () => void rules.push(projectRules()),
        resumeId: () => null,
      }),
      checkResolve: async () => ({ state: "not_running" }),
    });
    const clients = [client(), client()];
    for (const each of clients) app.connect(each);
    const send = (message: ClientMessage) => app.handleMessage(clients[0]!, message);

    const brand = clients[1]!.state.collections[0]!;
    expect(brand).toMatchObject({ kind: "brand", name: "Ferret Media", org: { member: true } });
    send({ type: "create_collection", kind: "series", name: "Podcast", parentId: brand.id });
    const series = clients[1]!.state.collections.find((collection) => collection.kind === "series")!;
    send({ type: "move_project", projectId: "ep12", collectionId: series.id });
    send({ type: "move_project", projectId: "ep11", collectionId: series.id });
    send({ type: "update_collection", id: brand.id, rules: "Never use the word 'synergy'." });

    expect(clients[1]!.state.collections.find((collection) => collection.id === series.id)!.projectIds).toEqual(["ep12", "ep11"]);
    expect(projects.projects.get("ep12")!.collection).toEqual({ id: series.id, name: "Podcast" });
    // Codex's AGENTS.md is rewritten straight away for both projects.
    expect(prepared.get(dir)).toContain("Never use the word 'synergy'.");
    expect(prepared.get("/projects/ep11")).toContain("Never use the word 'synergy'.");

    send({ type: "delete_collection", id: series.id });
    expect(clients[0]!.toasts).toEqual(['Series "Podcast" still has 2 projects in it. Move them out first.']);
    expect(clients[1]!.toasts).toEqual([]);

    // Add project puts the new project where it was asked, and refuses a place that's gone before adding anything.
    send({ type: "add_project", source: { kind: "cloud", name: "Promo" }, collectionId: series.id });
    const added = clients[1]!.state.projects.find((entry) => entry.name === "Promo")!;
    expect(projects.projects.get(added.id)!.collection).toEqual({ id: series.id, name: "Podcast" });
    send({ type: "add_project", source: { kind: "cloud", name: "Trailer" }, collectionId: "gone" });
    expect(clients[0]!.toasts.at(-1)).toMatch(/is gone/);
    expect(clients[1]!.state.projects.some((entry) => entry.name === "Trailer")).toBe(false);

    // PROJECT.md saved in the app reaches every client and Codex's AGENTS.md; one that is too long isn't saved.
    send({ type: "save_project_file", projectId: "ep12", text: "# Ep 12\n\n## Brief\nThe anniversary episode." });
    expect(clients[1]!.files.at(-1)).toEqual({ projectId: "ep12", file: path.join(dir, "PROJECT.md"), text: "# Ep 12\n\n## Brief\nThe anniversary episode.\n" });
    expect(prepared.get(dir)).toContain("## This project (PROJECT.md in the working folder)\n\n# Ep 12");
    send({ type: "save_project_file", projectId: "ep12", text: "x".repeat(20_001) });
    expect(clients[0]!.toasts.at(-1)).toMatch(/up to 20,000 characters/);
    expect(fs.readFileSync(path.join(dir, "PROJECT.md"), "utf8")).toMatch(/^# Ep 12/);

    // Claude gets them in its system prompt, read when the turn starts.
    send({ type: "select_thread", threadId: clients[0]!.state.projects.find((entry) => entry.id === "ep12")!.threads[0]!.id });
    send({ type: "send", text: "Cut the intro" });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(rules).toHaveLength(1);
    expect(rules[0]).toContain("Never use the word 'synergy'.");
  });
});

describe("skills imported in the app", () => {
  it("go to your own skills or a brand's, add to a skill of their name above, and are removed again", async () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-import-"));
    const shipped = path.join(root, "shipped");
    fs.mkdirSync(path.join(shipped, "sound"), { recursive: true });
    fs.writeFileSync(path.join(shipped, "sound", "SKILL.md"), "---\ndescription: Music and levels.\n---\n\nUnder speech.");
    const own = path.join(root, "skills");
    const app = createApp({
      version: "0.0.0",
      store: memoryProjectStore([]),
      collections: fileCollectionStore({ root }),
      linker: linkerWithOrgs([FERRET]),
      skills: [{ label: "Linker-Edit", dir: shipped }],
      ownSkills: own,
      createSession: () => ({ send: async () => {}, resumeId: () => null }),
      checkResolve: async () => ({ state: "not_running" }),
    });
    const viewer = client();
    app.connect(viewer);
    const brand = viewer.state.collections[0]!;
    const file = (filePath: string, text: string) => ({ path: filePath, base64: Buffer.from(text).toString("base64") });
    const importSkills = (id: string | null, files: { path: string; base64: string }[]) => viewer.importSkills(app, id, files);

    expect(await importSkills(null, [file("hooks.md", "---\ndescription: First seconds.\n---\n\nSell it."), file("pace/SKILL.md", "# Faster")])).toEqual({
      requestId: 1,
      result: { imported: ["hooks"], replaced: [], skipped: [{ path: "pace", reason: "It doesn't say what work it's for. Add a description to its front matter." }] },
    });
    expect(viewer.state.ownSkills).toEqual([{ name: "hooks", description: "First seconds.", file: path.join(own, "hooks", "SKILL.md") }]);
    expect(viewer.state.skills).toEqual([{ name: "sound", description: "Music and levels." }]);

    // In a brand, sound adds to Linker-Edit's and hooks to your own, so neither needs a description.
    expect(await importSkills(brand.id, [file("sound/SKILL.md", "# Louder"), file("hooks/SKILL.md", "Faster.")])).toMatchObject({
      result: { imported: ["hooks", "sound"], replaced: [], skipped: [] },
    });
    expect(viewer.state.collections[0]!.skills!.map((skill) => skill.name)).toEqual(["hooks", "sound"]);
    expect(await importSkills(brand.id, [file("sound/SKILL.md", "# Even louder")])).toMatchObject({ result: { replaced: ["sound"] } });

    expect(await importSkills(null, [{ path: "big.zip", base64: Buffer.alloc(9 * 1024 * 1024).toString("base64") }])).toMatchObject({
      result: { error: "That's 9 MB. Drop up to 8 MB at a time." },
    });
    expect(await importSkills("gone", [file("a.md", "A.")])).toMatchObject({ result: { error: expect.stringMatching(/is gone/) } });

    await viewer.removeSkill(app, null, "hooks");
    expect(viewer.state.ownSkills).toEqual([]);
    expect(viewer.state.collections[0]!.skills!.map((skill) => skill.name)).toEqual(["hooks", "sound"]);
  });
});

describe("skills synced with Linker", () => {
  const roles = { user_jane: { org_ferret: "admin" }, user_ann: { org_ferret: "editor" }, user_vic: { org_ferret: "viewer" } } as const;
  let fake: Awaited<ReturnType<typeof serveFakeLinkerSkills>>;
  beforeEach(async () => {
    fake = await serveFakeLinkerSkills(roles);
  });
  afterEach(() => fake.close());

  const file = (filePath: string, text: string) => ({ path: filePath, base64: Buffer.from(text).toString("base64") });
  const skill = (name: string, text: string) => file(`${name}/SKILL.md`, `---\ndescription: ${text}\n---\n\n${text}`);

  /** A computer of a person in Ferret Media, with a LinkerEdit folder of its own. */
  function computer(person: keyof typeof roles) {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-sync-"));
    const app = createApp({
      version: "0.0.0",
      store: memoryProjectStore([]),
      collections: fileCollectionStore({ root }),
      linker: linkerWithOrgs([FERRET], person),
      ownSkills: path.join(root, "skills"),
      linkerSkills: { api: createLinkerSkills({ appUrl: fake.url, accessToken: async () => person }), bin: path.join(root, "skills-bin") },
      createSession: () => ({ send: async () => {}, resumeId: () => null }),
      checkResolve: async () => ({ state: "not_running" }),
    });
    const viewer = client();
    app.connect(viewer);
    const brand = () => viewer.state.collections.find((collection) => collection.kind === "brand")!;
    return {
      root,
      app,
      viewer,
      brand,
      brandDir: path.join(root, "collections", "ferret-media", "skills"),
      importSkills: (id: string | null, files: { path: string; base64: string }[]) => viewer.importSkills(app, id, files).then((reply) => reply.result),
      removeSkill: (id: string | null, name: string) => viewer.removeSkill(app, id, name),
      own: () => viewer.state.ownSkills!.map((saved) => saved.name),
      brandSkills: () => brand().skills!.map((saved) => saved.name),
    };
  }

  it("your own reach your other computer, and one removed there is removed here", async () => {
    const [one, two] = [computer("user_jane"), computer("user_jane")];
    await Promise.all([one.app.syncSkills(), two.app.syncSkills()]);
    expect(two.viewer.state.ownSkillsSync).toEqual({ state: "synced", text: "Synced with Linker", skipped: [] });

    expect(await one.importSkills(null, [skill("hooks", "First seconds.")])).toEqual({ imported: ["hooks"], replaced: [], skipped: [] });
    await two.app.syncSkills();
    expect(two.own()).toEqual(["hooks"]);
    // Nothing changed since: Linker only says so.
    await two.app.syncSkills();
    expect(fake.state.requests.at(-1)).toBe("GET 304");

    await two.removeSkill(null, "hooks");
    await one.app.syncSkills();
    expect(one.own()).toEqual([]);
    expect(fake.state.requests.filter((request) => request.startsWith("DELETE"))).toEqual(["DELETE 204"]);
  });

  it("a brand's reach everyone in it; changed on two computers, Linker's wins and the other is kept in the bin", async () => {
    const [jane, ann] = [computer("user_jane"), computer("user_ann")];
    await jane.importSkills(jane.brand().id, [skill("sound", "Under speech.")]);
    await ann.app.syncSkills();
    expect(ann.brandSkills()).toEqual(["sound"]);
    expect(ann.brand().skillsSync).toEqual({ state: "synced", text: "Synced with Linker", skipped: [] });

    fs.writeFileSync(path.join(jane.brandDir, "sound", "SKILL.md"), "Jane's way.");
    fs.writeFileSync(path.join(ann.brandDir, "sound", "SKILL.md"), "Ann's way.");
    await jane.app.syncSkills();
    await ann.app.syncSkills();
    expect(fs.readFileSync(path.join(ann.brandDir, "sound", "SKILL.md"), "utf8")).toBe("Jane's way.");
    const bin = path.join(ann.root, "skills-bin");
    const binned = fs.readdirSync(bin, { recursive: true, encoding: "utf8" }).filter((found) => found.endsWith("SKILL.md"));
    expect(binned.map((found) => found.split(path.sep).slice(1).join("/"))).toEqual([expect.stringMatching(/^org_ferret\/sound-\d\d-\d\d-\d\d\/SKILL\.md$/)]);
    expect(fs.readFileSync(path.join(bin, binned[0]!), "utf8")).toBe("Ann's way.");
    await jane.app.syncSkills();
    expect(fs.readFileSync(path.join(jane.brandDir, "sound", "SKILL.md"), "utf8")).toBe("Jane's way.");
  });

  it("a viewer of a brand gets its skills, and is told it can't change them before anything is written", async () => {
    const [jane, vic] = [computer("user_jane"), computer("user_vic")];
    await jane.importSkills(jane.brand().id, [skill("sound", "Under speech.")]);
    await vic.app.syncSkills();
    expect(vic.brandSkills()).toEqual(["sound"]);

    expect(await vic.importSkills(vic.brand().id, [skill("hooks", "First seconds.")])).toEqual({ error: "Only admins and editors of Ferret Media can add its skills." });
    expect(fs.existsSync(path.join(vic.brandDir, "hooks"))).toBe(false);
    await vic.removeSkill(vic.brand().id, "sound");
    expect(vic.viewer.toasts.at(-1)).toBe("Only admins and editors of Ferret Media can remove its skills.");
    expect(vic.brandSkills()).toEqual(["sound"]);
  });

  it("keep an import on this computer while Linker can't take skills, and send it once it can", async () => {
    const [one, two] = [computer("user_jane"), computer("user_jane")];
    fake.state.unavailable = true;
    expect(await one.importSkills(null, [skill("hooks", "First seconds.")])).toMatchObject({ imported: ["hooks"] });
    await one.app.syncSkills();
    expect(one.viewer.state.ownSkillsSync).toEqual({ state: "problem", text: "Linker can't take skills yet", skipped: [] });

    fake.state.unavailable = false;
    await one.app.syncSkills();
    expect(one.viewer.state.ownSkillsSync!.state).toBe("synced");
    await two.app.syncSkills();
    expect(two.own()).toEqual(["hooks"]);
  });

  it("say a sub-brand's stay on this computer", () => {
    const one = computer("user_jane");
    one.app.handleMessage(one.viewer, { type: "create_collection", kind: "subbrand", name: "Shorts", parentId: one.brand().id });
    const shorts = one.viewer.state.collections.find((collection) => collection.name === "Shorts")!;
    expect(shorts.skillsSync).toEqual({ state: "local", text: "On this computer only, like the sub-brand itself", skipped: [] });
  });
});

function client() {
  let state: AppState | null = null;
  const toasts: string[] = [];
  const files: { projectId: string; file: string; text: string }[] = [];
  const imports: Omit<Extract<ServerMessage, { type: "skills_imported" }>, "type">[] = [];
  // Who waits for the next message of a type.
  const waiting = new Map<string, () => void>();
  const next = (type: ServerMessage["type"]) => new Promise<void>((resolve) => waiting.set(type, resolve));
  return {
    toasts,
    files,
    imports,
    /** Sends import_skills; its answer. */
    async importSkills(app: App, id: string | null, files: { path: string; base64: string }[]) {
      const answered = next("skills_imported");
      app.handleMessage(this, { type: "import_skills", requestId: imports.length + 1, id, files });
      await answered;
      return imports.at(-1)!;
    },
    /** Sends remove_skill, and waits for the list without it, or the toast saying why not. */
    async removeSkill(app: App, id: string | null, name: string) {
      const done = Promise.race([next("meta"), next("toast")]);
      app.handleMessage(this, { type: "remove_skill", id, name });
      await done;
    },
    send(data: string) {
      const message = JSON.parse(data) as ServerMessage;
      waiting.get(message.type)?.();
      waiting.delete(message.type);
      if (message.type === "toast") toasts.push(message.text);
      if (message.type === "project_file") files.push({ projectId: message.projectId, file: message.file, text: message.text });
      if (message.type === "skills_imported") imports.push({ requestId: message.requestId, result: message.result });
      state = applyServerMessage(state, message);
    },
    get state(): AppState {
      if (!state) throw new Error("No snapshot yet");
      return state;
    },
  };
}
