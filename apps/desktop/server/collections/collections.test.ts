import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import type { AppState, ClientMessage, ServerMessage } from "../../shared/protocol.ts";
import { applyServerMessage } from "../../src/lib/appState.ts";
import { createApp } from "../app.ts";
import type { ProjectCollection, SavedProject } from "../projects/store.ts";
import { memoryProjectStore } from "../projects/testing.ts";
import { EDITING_RULES } from "../rules.ts";
import { createCollections } from "./collections.ts";
import { memoryCollectionStore } from "./testing.ts";

function setup(projects: SavedProject[] = []) {
  const store = memoryCollectionStore();
  const saved = new Map(projects.map((project) => [project.id, project]));
  const commits: string[] = [];
  let changes = 0;
  const collections = createCollections({
    store,
    projects: () => [...saved.values()],
    setCollection(projectId, collection: ProjectCollection | null, commitMessage) {
      saved.set(projectId, { ...saved.get(projectId)!, collection });
      commits.push(commitMessage);
    },
    removeProject: (projectId) => saved.delete(projectId),
    changed: () => (changes += 1),
  });
  const send = (message: ClientMessage) => collections.handle(message);
  const byName = (name: string) => collections.summaries().find((collection) => collection.name === name)!;
  return { collections, store, saved, commits, send, byName, changes: () => changes };
}

const project = (id: string, dir = `/projects/${id}`): SavedProject => ({ id, dir, name: id, link: null, local: {}, icon: null, pinned: false, archived: false });

describe("workspaces and series", () => {
  it("nests one level deep: a series in a workspace, nothing in a series, a workspace in nothing", () => {
    const { send, byName, collections } = setup();
    expect(send({ type: "create_collection", kind: "workspace", name: "  Ferret   Media " })).toBeNull();
    const workspace = byName("Ferret Media");
    expect(workspace).toMatchObject({ kind: "workspace", parentId: null, rules: "", projectIds: [], icon: { kind: "monogram" } });

    expect(send({ type: "create_collection", kind: "series", name: "Podcast", parentId: workspace.id })).toBeNull();
    const series = byName("Podcast");
    expect(series.parentId).toBe(workspace.id);

    expect(send({ type: "create_collection", kind: "series", name: "Clips", parentId: series.id })).toMatch(/only go in a workspace/);
    expect(send({ type: "create_collection", kind: "workspace", name: "Sub", parentId: workspace.id })).toBeNull();
    // A workspace's parent is ignored on create, and refused on update.
    expect(byName("Sub").parentId).toBeNull();
    expect(send({ type: "update_collection", id: byName("Sub").id, parentId: workspace.id })).toMatch(/can't go inside/);
    expect(send({ type: "update_collection", id: series.id, parentId: byName("Sub").id })).toBeNull();
    expect(byName("Podcast").parentId).toBe(byName("Sub").id);
    expect(send({ type: "update_collection", id: series.id, parentId: series.id })).toMatch(/only go in a workspace/);

    // A series on its own is fine too.
    expect(send({ type: "create_collection", kind: "series", name: "Shorts" })).toBeNull();
    expect(byName("Shorts").parentId).toBeNull();
    expect(send({ type: "create_collection", kind: "series", name: "   " })).toMatch(/name/);
    // Workspaces first.
    expect(collections.summaries().map((collection) => collection.name)).toEqual(["Ferret Media", "Sub", "Podcast", "Shorts"]);
  });

  it("refuses to delete a workspace or series that still has projects or series in it", () => {
    const { send, byName, store, collections } = setup([project("ep12"), project("ep11")]);
    send({ type: "create_collection", kind: "workspace", name: "Ferret Media" });
    const workspace = byName("Ferret Media");
    send({ type: "create_collection", kind: "series", name: "Podcast", parentId: workspace.id });
    const series = byName("Podcast");
    send({ type: "move_project", projectId: "ep12", collectionId: series.id });
    send({ type: "move_project", projectId: "ep11", collectionId: series.id });

    expect(send({ type: "delete_collection", id: workspace.id })).toBe('Workspace "Ferret Media" still has 1 series in it. Move it out first.');
    expect(send({ type: "delete_collection", id: series.id })).toBe('Series "Podcast" still has 2 projects in it. Move them out first.');
    expect(store.collections.size).toBe(2);

    send({ type: "move_project", projectId: "ep12", collectionId: null });
    send({ type: "move_project", projectId: "ep11", collectionId: workspace.id });
    expect(send({ type: "delete_collection", id: series.id })).toBeNull();
    expect(send({ type: "delete_collection", id: workspace.id })).toMatch(/still has 1 project in it/);
    expect(collections.summaries().map((collection) => collection.name)).toEqual(["Ferret Media"]);
    expect(store.collections.size).toBe(1);
  });

  it("puts projects in a collection and keeps its name in them when it's renamed", () => {
    const { send, byName, saved, commits, changes } = setup([project("ep12")]);
    send({ type: "create_collection", kind: "series", name: "Podcast" });
    const series = byName("Podcast");
    send({ type: "move_project", projectId: "ep12", collectionId: series.id });
    expect(saved.get("ep12")!.collection).toEqual({ id: series.id, name: "Podcast" });
    expect(byName("Podcast").projectIds).toEqual(["ep12"]);

    send({ type: "update_collection", id: series.id, name: "The Podcast", rules: "Keep it short." });
    expect(saved.get("ep12")!.collection).toEqual({ id: series.id, name: "The Podcast" });
    expect(byName("The Podcast").rules).toBe("Keep it short.");
    expect(send({ type: "move_project", projectId: "ep12", collectionId: "gone" })).toMatch(/is gone/);
    expect(commits).toEqual(['Put the project in Series "Podcast"', 'Series "The Podcast" renamed']);
    expect(changes()).toBe(3);
  });

  it("refuses an icon that isn't one", () => {
    const { send, byName } = setup();
    send({ type: "create_collection", kind: "workspace", name: "W", icon: { kind: "emoji", emoji: "🎙", color: "pink" } });
    const id = byName("W").id;
    send({ type: "update_collection", id, icon: { kind: "lucide", name: "<script>", color: "blue" } });
    send({ type: "update_collection", id, icon: { kind: "monogram", text: "AB", color: "nope" } as never });
    expect(byName("W").icon).toEqual({ kind: "emoji", emoji: "🎙", color: "pink" });
    send({ type: "update_collection", id, icon: { kind: "lucide", name: "mic", color: "teal" } });
    expect(byName("W").icon).toEqual({ kind: "lucide", name: "mic", color: "teal" });
  });

  it("stacks the agent's rules: editing rules, workspace, series, then the project's own, most specific last", () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-rules-"));
    fs.writeFileSync(path.join(dir, "RULES.md"), "Ep 12 is the anniversary episode.\n");
    const { send, byName, collections, saved } = setup([project("ep12", dir), project("ep11")]);
    send({ type: "create_collection", kind: "workspace", name: "Ferret Media" });
    send({ type: "update_collection", id: byName("Ferret Media").id, rules: "End on the logo." });
    send({ type: "create_collection", kind: "series", name: "Podcast", parentId: byName("Ferret Media").id });
    send({ type: "update_collection", id: byName("Podcast").id, rules: "Lower thirds in Inter Bold." });
    send({ type: "move_project", projectId: "ep12", collectionId: byName("Podcast").id });
    send({ type: "move_project", projectId: "ep11", collectionId: byName("Podcast").id });

    const rules = collections.rulesFor(saved.get("ep12")!);
    expect(rules.startsWith(EDITING_RULES)).toBe(true);
    const order = [
      "# The user's rules",
      '## Workspace "Ferret Media" (its RULES.md)\n\nEnd on the logo.',
      '## Series "Podcast" (its RULES.md)\n\nLower thirds in Inter Bold.',
      "## This project's notes (RULES.md in the working folder)\n\nEp 12 is the anniversary episode.",
    ].map((part) => rules.indexOf(part));
    expect(order.every((index, i) => index > 0 && (i === 0 || index > order[i - 1]!))).toBe(true);

    // Without notes of its own, the project gets the workspace's and series' rules only.
    const other = collections.rulesFor(saved.get("ep11")!);
    expect(other).toContain("End on the logo.");
    expect(other).toContain("Lower thirds in Inter Bold.");
    expect(other).not.toContain("This project's notes");

    // A project in no collection, with no notes, gets the editing rules alone.
    expect(collections.rulesFor(project("loose"))).toBe(EDITING_RULES);
  });
});

describe("workspaces and series in the app", () => {
  it("sends collections to every client and gives both agents the project's rules at each turn", async () => {
    const projects = memoryProjectStore([{ project: project("ep12") }, { project: project("ep11") }]);
    const prepared = new Map<string, string>();
    const rules: string[] = [];
    const app = createApp({
      version: "0.0.0",
      store: projects,
      collections: memoryCollectionStore(),
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

    send({ type: "create_collection", kind: "workspace", name: "Ferret Media" });
    const workspace = clients[1]!.state.collections[0]!;
    send({ type: "create_collection", kind: "series", name: "Podcast", parentId: workspace.id });
    const series = clients[1]!.state.collections.find((collection) => collection.kind === "series")!;
    send({ type: "move_project", projectId: "ep12", collectionId: series.id });
    send({ type: "move_project", projectId: "ep11", collectionId: series.id });
    send({ type: "update_collection", id: workspace.id, rules: "Never use the word 'synergy'." });

    expect(clients[1]!.state.collections.find((collection) => collection.id === series.id)!.projectIds).toEqual(["ep12", "ep11"]);
    expect(projects.projects.get("ep12")!.collection).toEqual({ id: series.id, name: "Podcast" });
    // Codex's AGENTS.md is rewritten straight away for both projects.
    expect(prepared.get("/projects/ep12")).toContain("Never use the word 'synergy'.");
    expect(prepared.get("/projects/ep11")).toContain("Never use the word 'synergy'.");

    send({ type: "delete_collection", id: workspace.id });
    expect(clients[0]!.toasts).toEqual(['Workspace "Ferret Media" still has 1 series in it. Move it out first.']);
    expect(clients[1]!.toasts).toEqual([]);

    // Claude gets them in its system prompt, read when the turn starts.
    send({ type: "send", text: "Cut the intro" });
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(rules).toHaveLength(1);
    expect(rules[0]).toContain("Never use the word 'synergy'.");
  });
});

function client() {
  let state: AppState | null = null;
  const toasts: string[] = [];
  return {
    toasts,
    send(data: string) {
      const message = JSON.parse(data) as ServerMessage;
      if (message.type === "toast") toasts.push(message.text);
      state = applyServerMessage(state, message);
    },
    get state(): AppState {
      if (!state) throw new Error("No snapshot yet");
      return state;
    },
  };
}
