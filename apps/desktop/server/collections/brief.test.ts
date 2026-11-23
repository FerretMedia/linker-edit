import { describe, expect, it } from "vitest";
import type { AppState, ClientMessage, ServerMessage } from "../../shared/protocol.ts";
import { applyServerMessage } from "../../src/lib/appState.ts";
import { createApp, type App } from "../app.ts";
import type { SavedProject } from "../projects/store.ts";
import { memoryProjectStore } from "../projects/testing.ts";
import { AUTONOMOUS_RULES, EDITING_RULES, LEARNING_RULES } from "../rules.ts";
import { memoryCollectionStore } from "./testing.ts";

type Script = (app: App) => Promise<void>;

const project = (id: string): SavedProject => ({ id, dir: `/projects/${id}`, name: id, link: null, local: {}, icon: null, pinned: false, archived: false });

/** An app whose agent turns run the scripts in order, each calling the agents' tools as an MCP server would. */
function setup(...scripts: Script[]) {
  const projects = memoryProjectStore([{ project: project("ep12") }]);
  const collections = memoryCollectionStore();
  const prompts: string[] = [];
  const rules: string[] = [];
  const resolveCalls: string[] = [];
  const app: App = createApp({
    version: "0.0.0",
    store: projects,
    collections,
    createSession: (_provider, _mode, _dir, _resumeId, projectRules) => ({
      send: async (text) => {
        prompts.push(text);
        rules.push(projectRules());
        await scripts.shift()?.(app);
      },
      resumeId: () => null,
    }),
    checkResolve: async () => ({ state: "not_running" }),
    resolveTools: async (tool) => {
      resolveCalls.push(tool);
      return { text: "{}" };
    },
    applyTimeline: async (_dir, timelineId) => ({ main: { id: timelineId, name: "Edit" }, previous: { id: "t1", name: "Edit (before AI)" }, was: "AI - Tight" }),
  });
  let state: AppState | null = null;
  const waiters: { check: () => boolean; resolve: () => void }[] = [];
  const client = {
    toasts: [] as string[],
    send(data: string) {
      const message = JSON.parse(data) as ServerMessage;
      if (message.type === "toast") client.toasts.push(message.text);
      state = applyServerMessage(state, message);
      for (const waiter of [...waiters]) {
        if (waiter.check()) {
          waiters.splice(waiters.indexOf(waiter), 1);
          waiter.resolve();
        }
      }
    },
    get state(): AppState {
      return state!;
    },
    until(predicate: (state: AppState) => boolean): Promise<void> {
      const check = () => state !== null && predicate(state);
      return check() ? Promise.resolve() : new Promise((resolve) => waiters.push({ check, resolve }));
    },
  };
  app.connect(client);
  const send = (message: ClientMessage) => app.handleMessage(client, message);
  const byName = (name: string) => client.state.collections.find((collection) => collection.name === name)!;
  /** Answers the approval that is waiting, and gives back what it showed. */
  async function answer(allow: boolean) {
    await client.until((current) => current.approvals.length > 0);
    const request = client.state.approvals[0]!;
    send({ type: "approve", id: request.id, allow });
    return request;
  }
  const turnDone = () => client.until((current) => !current.busy);
  return { app, client, send, byName, answer, turnDone, prompts, rules, resolveCalls, projects, collections };
}

describe("a setup thread", () => {
  it("opens once per workspace, as a project of its own that isn't counted among the workspace's projects", () => {
    const { client, send, byName, projects } = setup();
    send({ type: "create_collection", kind: "workspace", name: "Ferret Media" });
    send({ type: "setup_collection", id: byName("Ferret Media").id });

    const setupProject = client.state.projects.find((candidate) => candidate.setup)!;
    expect(setupProject).toMatchObject({ name: "Ferret Media setup", link: null });
    expect(client.state.projectId).toBe(setupProject.id);
    expect(byName("Ferret Media")).toMatchObject({ setupProjectId: setupProject.id, projectIds: [] });
    expect(projects.projects.get(setupProject.id)).toMatchObject({ setup: true, collection: { name: "Ferret Media" } });

    // Asked again, the same thread opens; a rename shows in its name.
    const threadId = client.state.threadId;
    send({ type: "select_thread", threadId: client.state.projects.find((candidate) => !candidate.setup)!.threads[0]!.id });
    send({ type: "setup_collection", id: byName("Ferret Media").id });
    expect(client.state.threadId).toBe(threadId);
    expect(client.state.projects.filter((candidate) => candidate.setup)).toHaveLength(1);
    send({ type: "update_collection", id: byName("Ferret Media").id, name: "Ferret" });
    expect(client.state.projects.find((candidate) => candidate.setup)!.name).toBe("Ferret setup");

    // It stays with its workspace, and goes when the workspace is deleted.
    send({ type: "move_project", projectId: setupProject.id, collectionId: null });
    expect(client.toasts).toEqual(["Setup threads stay with what they set up."]);
    send({ type: "delete_collection", id: byName("Ferret").id });
    expect(client.state.collections).toEqual([]);
    expect(client.state.projects.map((candidate) => candidate.id)).toEqual(["ep12"]);
  });

  it("gets the setup assistant's rules, can't use Resolve outside a visit, and changes the brief only when the user accepts", async () => {
    const replies: { text: string; isError: boolean }[] = [];
    const { app, client, send, byName, answer, turnDone, rules, resolveCalls } = setup(async (running) => {
      replies.push(await running.resolveTool("list_media", {}));
      replies.push(await running.resolveTool("view_frames", { timeline: "Edit", frames: [1] }));
      replies.push(await running.resolveTool("view_frames", { file: "/videos/reel.mp4", seconds: [1] }));
      replies.push(await running.libraryTool("library_import", { files: ["/music/a.wav"] }));
      replies.push(await running.briefTool("save_example", { name: "Reel 1.mp4", notes: "34 seconds, 9:16." }));
      replies.push(await running.briefTool("save_brief", { rules: "Captions in white.", summary: "First brief." }));
      replies.push(await running.briefTool("save_brief", { rules: "End on the logo.", summary: "From the examples." }));
      replies.push(await running.briefTool("save_brief", { for: "series", series: "Reels", rules: "30 to 40 seconds, 9:16.", summary: "The reels format." }));
      replies.push(await running.briefTool("save_brief", { rules: "   ", summary: "Nothing." }));
    });
    send({ type: "create_collection", kind: "workspace", name: "Ferret Media" });
    send({ type: "setup_collection", id: byName("Ferret Media").id });
    send({ type: "send", text: "My reels are in D:/reels" });

    // Full access is on, and the brief still waits for the user: also when full access is switched on while it waits.
    expect(client.state.approvalMode).toBe("auto");
    await client.until((current) => current.approvals.length > 0);
    send({ type: "set_approval_mode", mode: "ask" });
    send({ type: "set_approval_mode", mode: "auto" });
    expect(client.state.approvals).toHaveLength(1);
    expect((await answer(false)).input).toEqual({ for: 'Workspace "Ferret Media"', summary: "First brief.", was: "", rules: "Captions in white." });
    expect(byName("Ferret Media").rules).toBe("");
    expect(await answer(true)).toMatchObject({ tool: "mcp__linker__save_brief" });
    expect((await answer(true)).input).toMatchObject({ for: 'New series "Reels"', was: "" });
    await turnDone();

    expect(rules[0]).toMatch(/^You are Linker-Edit's setup assistant\. You are learning how Workspace "Ferret Media" is edited/);
    expect(rules[0]).not.toContain(EDITING_RULES);
    // Only the frames of a file got through to the Resolve tools.
    expect(resolveCalls).toEqual(["view_frames"]);
    expect(replies.map((reply) => reply.isError)).toEqual([true, true, false, true, false, true, false, false, true]);
    expect(replies[0]!.text).toMatch(/setup thread/);
    expect(replies[5]!.text).toMatch(/declined/);
    expect(byName("Ferret Media")).toMatchObject({ rules: "End on the logo.\n", examples: [{ name: "reel-1" }] });
    expect(byName("Reels")).toMatchObject({ kind: "series", parentId: byName("Ferret Media").id, rules: "30 to 40 seconds, 9:16.\n" });

    // The next turn's assistant sees the brief as it is now.
    expect(app.busy()).toBe(false);
    send({ type: "send", text: "Anything else?" });
    await client.until((current) => rules.length === 2 && !current.busy);
    expect(rules[1]).toContain('## Series "Reels"\n\n30 to 40 seconds, 9:16.');
    expect(rules[1]).toContain("Notes on 1 example video are in");
  });
});

describe("learning from feedback", () => {
  async function inSeries(...scripts: Script[]) {
    const harness = setup(...scripts);
    const { send, byName } = harness;
    send({ type: "create_collection", kind: "workspace", name: "Ferret Media" });
    send({ type: "create_collection", kind: "series", name: "Reels", parentId: byName("Ferret Media").id });
    send({ type: "move_project", projectId: "ep12", collectionId: byName("Reels").id });
    return harness;
  }

  it("shows a lesson next to the user's own words, and keeps it only when accepted", async () => {
    const replies: { text: string; isError: boolean }[] = [];
    const { client, send, byName, answer, turnDone, rules } = await inSeries(
      async (running) => {
        replies.push(await running.briefTool("add_lesson", { rule: "Intro at most 3 seconds.", feedback: "the start drags" }));
        replies.push(await running.briefTool("add_lesson", { for: "workspace", rule: "Never use the blue logo.", feedback: "wrong logo again" }));
        replies.push(await running.briefTool("add_lesson", { rule: "Shorter.", feedback: "" }));
        // A lesson never makes a series; only new rules do.
        replies.push(await running.briefTool("add_lesson", { for: "series", series: "Vlog", rule: "No music.", feedback: "no music in vlogs" }));
      },
      async () => {},
    );
    send({ type: "send", text: "the start drags" });
    expect((await answer(true)).input).toEqual({ for: 'Series "Reels"', feedback: "the start drags", rule: "Intro at most 3 seconds." });
    expect((await answer(false)).input).toMatchObject({ for: 'Workspace "Ferret Media"' });
    await turnDone();

    expect(replies.map((reply) => reply.isError)).toEqual([false, true, true, true]);
    expect(replies[3]!.text).toMatch(/no series "Vlog"/);
    expect(byName("Reels").lessons).toMatchObject([{ rule: "Intro at most 3 seconds.", feedback: "the start drags", project: "ep12" }]);
    expect(byName("Ferret Media").lessons).toEqual([]);

    // The next turn follows it, until the user removes it.
    send({ type: "send", text: "next" });
    await client.until((current) => rules.length === 2 && !current.busy);
    expect(rules[1]).toContain("- Intro at most 3 seconds.");
    expect(rules[1]!.endsWith(LEARNING_RULES)).toBe(true);
    send({ type: "remove_lesson", id: byName("Reels").id, lessonId: byName("Reels").lessons[0]!.id });
    expect(byName("Reels").lessons).toEqual([]);
  });

  it("refuses the brief tools outside a turn and for a project with no workspace or series", async () => {
    const replies: { text: string; isError: boolean }[] = [];
    const { app, send, turnDone } = setup(async (running) => {
      replies.push(await running.briefTool("add_lesson", { rule: "Intro at most 3 seconds.", feedback: "the start drags" }));
    });
    expect(await app.briefTool("save_brief", { rules: "x", summary: "y" })).toMatchObject({ isError: true, text: expect.stringMatching(/No Linker-Edit turn/) });
    send({ type: "send", text: "hi" });
    await turnDone();
    expect(replies[0]).toMatchObject({ isError: true, text: expect.stringMatching(/isn't in a workspace or series/) });
  });

  it("tells the agent to work on its own only where that is switched on", async () => {
    const { client, send, byName, rules } = await inSeries(
      async () => {},
      async () => {},
    );
    send({ type: "send", text: "footage is in D:/shoot" });
    await client.until((current) => rules.length === 1 && !current.busy);
    expect(rules[0]).not.toContain(AUTONOMOUS_RULES);
    send({ type: "update_collection", id: byName("Reels").id, autonomous: true });
    send({ type: "send", text: "and D:/shoot2" });
    await client.until((current) => rules.length === 2 && !current.busy);
    expect(rules[1]).toContain(AUTONOMOUS_RULES);
  });

  it("counts the rounds of feedback when a timeline is approved, and has the agent learn from the edit", async () => {
    const { client, send, byName, turnDone, prompts } = await inSeries(
      async () => {},
      async () => {},
      async () => {},
      async () => {},
    );
    for (const text of ["Footage is in D:/shoot", "0:14 is too slow", "Wrong song"]) {
      send({ type: "send", text });
      await client.until((current) => current.items.some((item) => item.kind === "user" && item.text === text) && !current.busy);
    }
    send({ type: "apply_timeline", projectId: "ep12", timelineId: "t2" });
    await client.until(() => prompts.length === 4);
    await turnDone();

    expect(prompts[3]).toContain("Learn from this edit:");
    expect(client.state.items.some((item) => item.kind === "notice" && item.text.startsWith("Approved:"))).toBe(true);
    // Two messages of feedback after the one that started the edit.
    expect(byName("Reels").edits).toMatchObject([{ project: "ep12", rounds: 2 }]);
  });
});
