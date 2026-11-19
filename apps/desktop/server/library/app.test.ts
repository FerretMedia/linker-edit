// The library inside the app: the page's messages, and the agents' tools against the stand-in Resolve
// (server/resolve/fixtures/DaVinciResolveScript.py), running Linker-Edit's real scripts.
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import type { LibraryView, ResolveStatus, ServerMessage } from "../../shared/protocol.ts";
import { createApp, type AppOptions } from "../app.ts";
import { memoryProjectStore, workspaceProject } from "../projects/testing.ts";
import { runResolveTool } from "../resolve/run.ts";
import { hasPython, pythonConnection } from "../resolve/testing.ts";
import { memoryLibraryStore, projectFolders } from "./folders.ts";
import { createLibrary } from "./library.ts";
import type { ProbeFile } from "./scan.ts";

const FAKE = fs.readFileSync(path.join(import.meta.dirname, "..", "resolve", "fixtures", "DaVinciResolveScript.py"), "utf8");
const temp = (name: string) => fs.mkdtempSync(path.join(os.tmpdir(), `linker-edit-library-app-${name}-`));
const probe: ProbeFile = async () => ({ duration: 30, channels: 2, loudness: -14 });

function write(file: string) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, "x");
}

function setup(options: Partial<AppOptions> = {}) {
  const library = createLibrary({ store: memoryLibraryStore(), cacheDir: temp("cache"), probe });
  const store = memoryProjectStore([{ project: { ...workspaceProject, name: "Doc" } }]);
  let release!: () => void;
  const turnEnds = new Promise<void>((resolve) => (release = resolve));
  let started!: () => void;
  const turnStarted = new Promise<void>((resolve) => (started = resolve));
  const app = createApp({
    version: "0.0.0",
    store,
    // Each turn waits until the test lets it end, so the agent's tools can be called while it runs.
    createSession: () => ({
      send: async () => {
        started();
        await turnEnds;
      },
      resumeId: () => null,
    }),
    checkResolve: async (): Promise<ResolveStatus> => ({
      state: "connected",
      product: "DaVinci Resolve Studio",
      version: "21",
      project: "Doc",
      projectId: "doc-id",
      database: { type: "Disk", name: "Local Database" },
      timeline: "Edit",
    }),
    library,
    ...options,
  });
  const views: LibraryView[] = [];
  const toasts: string[] = [];
  const client = {
    send(data: string) {
      const message = JSON.parse(data) as ServerMessage;
      if (message.type === "library_view") views.push(message.view);
      if (message.type === "toast") toasts.push(message.text);
    },
  };
  app.connect(client);
  const send = (message: Parameters<typeof app.handleMessage>[1]) => app.handleMessage(client, message);
  return { app, library, store, send, views, toasts, turnStarted, endTurn: release, view: () => views.at(-1)! };
}

const until = async (check: () => boolean) => {
  for (let i = 0; i < 200 && !check(); i += 1) await new Promise((resolve) => setTimeout(resolve, 10));
  expect(check()).toBe(true);
};

describe("the library page", () => {
  it("adds folders to the universal library and the project's, scans them, and keeps what's noted on files", async () => {
    const music = temp("music");
    const own = temp("own");
    write(path.join(music, "Upbeat.wav"));
    write(path.join(music, "Calm.wav"));
    write(path.join(own, "Theme.wav"));
    const { send, view, toasts, store, library } = setup();

    send({ type: "library_open", scope: { kind: "universal" }, query: {} });
    expect(view()).toMatchObject({ scope: { kind: "universal" }, folders: [], files: [], total: 0 });
    expect(view().scopes.map((scope) => scope.label)).toEqual(["Universal", "Doc"]);

    send({ type: "library_add_folder", scope: { kind: "universal" }, folder: { kind: "music", label: "Songs", path: music, note: "YouTube only" } });
    await until(() => view().files.length === 2);
    expect(view().folders).toMatchObject([{ label: "Songs", note: "YouTube only", files: 2, scanning: null, problem: null }]);
    const folderId = view().folders[0]!.id;

    send({ type: "library_add_folder", scope: { kind: "universal" }, folder: { kind: "music", label: "Again", path: music, note: "" } });
    expect(toasts.at(-1)).toContain('already in this library as "Songs"');
    send({ type: "library_add_folder", scope: { kind: "universal" }, folder: { kind: "music", label: "Gone", path: path.join(music, "nope"), note: "" } });
    expect(toasts.at(-1)).toContain("There is no folder");

    send({ type: "library_update_file", folderId, rel: "Upbeat.wav", star: true, tags: ["happy", " Happy ", "intro"], note: "for the opener" });
    expect(view().files[0]).toMatchObject({ name: "Upbeat.wav", star: true, tags: ["happy", "intro"], note: "for the opener" });
    send({ type: "library_open", scope: { kind: "universal" }, query: { query: "opener" } });
    expect(view().files.map((file) => file.name)).toEqual(["Upbeat.wav"]);
    // Only paths inside the folder.
    send({ type: "library_update_file", folderId, rel: "../Upbeat.wav", star: true });

    send({ type: "library_open", scope: { kind: "project", projectId: "workspace" }, query: {} });
    send({ type: "library_add_folder", scope: { kind: "project", projectId: "workspace" }, folder: { kind: "music", label: "Ours", path: own, note: "" } });
    await until(() => view().files.length === 1);
    expect(view()).toMatchObject({ scope: { kind: "project", projectId: "workspace" }, folders: [{ label: "Ours" }] });
    // Kept in the project's local.json, on this machine only.
    expect(projectFolders(store.projects.get("workspace")!.local).map((folder) => folder.path)).toEqual([path.resolve(own)]);

    send({ type: "library_remove_folder", scope: { kind: "universal" }, folderId });
    expect(library.folders()).toEqual([]);
    // Removing a folder forgets it; its files stay.
    expect(fs.existsSync(path.join(music, "Upbeat.wav"))).toBe(true);
  });

  it("sorts an assets folder for the user to check, then adds the folders they chose at once", async () => {
    const assets = temp("assets");
    write(path.join(assets, "Music", "Song.wav"));
    write(path.join(assets, "SoundEffects", "Whoosh.wav"));
    write(path.join(assets, "Misc", "notes.txt"));
    const { app, send, view, toasts } = setup();
    const replies: ServerMessage[] = [];
    const client = { send: (data: string) => void replies.push(JSON.parse(data) as ServerMessage) };
    app.connect(client);

    app.handleMessage(client, { type: "library_sort_folder", requestId: 7, path: assets });
    await until(() => replies.some((reply) => reply.type === "library_sorted"));
    const sorted = replies.find((reply) => reply.type === "library_sorted")!;
    expect(sorted).toMatchObject({ requestId: 7, sort: { root: path.resolve(assets), rootKind: null } });
    const sort = (sorted as Extract<ServerMessage, { type: "library_sorted" }>).sort;
    if ("error" in sort) throw new Error(sort.error);
    expect(sort.folders.map((folder) => [folder.name, folder.kind])).toEqual([
      ["Misc", null],
      ["Music", "music"],
      ["SoundEffects", "sfx"],
    ]);

    send({ type: "library_open", scope: { kind: "universal" }, query: {} });
    send({
      type: "library_add_folders",
      scope: { kind: "universal" },
      folders: [
        { kind: "music", label: "Music", path: path.join(assets, "Music"), note: "" },
        { kind: "sfx", label: "SoundEffects", path: path.join(assets, "SoundEffects"), note: "" },
        { kind: "sfx", label: "Again", path: path.join(assets, "SoundEffects"), note: "" },
      ],
    });
    // The two new ones are added together; the one already there is said, not added twice.
    expect(view().folders.map((folder) => [folder.label, folder.kind])).toEqual([
      ["Music", "music"],
      ["SoundEffects", "sfx"],
    ]);
    expect(toasts.at(-1)).toContain('already in this library as "SoundEffects"');
    await until(() => view().files.length === 2);

    app.handleMessage(client, { type: "library_sort_folder", requestId: 8, path: path.join(assets, "missing") });
    await until(() => replies.some((reply) => reply.type === "library_sorted" && reply.requestId === 8));
    expect(replies.find((reply) => reply.type === "library_sorted" && reply.requestId === 8)).toMatchObject({ sort: { error: expect.stringContaining("There is no folder") } });
  });
});

describe.skipIf(!hasPython)("the library's agent tools with the stand-in Resolve", () => {
  it("imports into Library/Music, reuses a clip already in the media pool, and notes where files were used", async () => {
    const music = temp("import-music");
    const sfx = temp("import-sfx");
    const song = path.join(music, "Song.wav");
    const fresh = path.join(music, "Fresh.wav");
    const whoosh = path.join(sfx, "Whoosh.wav");
    for (const file of [song, fresh, whoosh]) write(file);
    const outside = path.join(temp("outside"), "Other.wav");
    write(outside);

    // Song.wav is in the media pool already, in another bin.
    const stateFile = path.join(temp("resolve"), "state.json");
    fs.writeFileSync(
      stateFile,
      JSON.stringify({
        project: { name: "Doc", id: "doc-id", current: "Edit", timelines: [{ id: "t1", name: "Edit", video: [[]], audio: [[]] }] },
        media: { name: "Master", clips: [], folders: [{ name: "Music", clips: [{ id: "m-song", name: "Song.wav", path: song, props: { Type: "Audio" } }] }] },
      }),
    );
    const connection = pythonConnection(FAKE, { FAKE_RESOLVE: stateFile });
    const calls = () =>
      fs.existsSync(`${stateFile}.log`) ? fs.readFileSync(`${stateFile}.log`, "utf8").trim().split("\n").map((line) => JSON.parse(line) as unknown[]) : [];
    const backups: string[] = [];
    const { app, send, library, turnStarted, endTurn } = setup({
      resolveTools: (tool, input, dir) => runResolveTool(connection, tool, input, { dir }),
      backupProject: async (dir) => {
        backups.push(dir);
        return path.join(dir, "backups", "1.drp");
      },
    });
    library.saveFolders([
      { id: "music", kind: "music", label: "Songs", path: music, note: "Licensed for YouTube only" },
      { id: "sfx", kind: "sfx", label: "Whooshes", path: sfx, note: "" },
    ]);
    await app.scanLibrary();

    // Searching only reads, so it works between turns too.
    const search = await app.libraryTool("library_search", { kind: "music" });
    expect(JSON.parse(search.text)).toEqual({
      results: [
        { path: fresh, kind: "music", library: "universal", folder: "Songs", folder_note: "Licensed for YouTube only", seconds: 30, channels: 2, loudness_lufs: -14 },
        { path: song, kind: "music", library: "universal", folder: "Songs", folder_note: "Licensed for YouTube only", seconds: 30, channels: 2, loudness_lufs: -14 },
      ],
      total: 2,
    });
    expect(await app.libraryTool("library_import", { files: [song] })).toEqual({
      text: "No Linker-Edit turn is running, so Resolve calls are refused.",
      isError: true,
    });

    send({ type: "send", text: "put an upbeat song under it and a whoosh on every cut" });
    await turnStarted;
    const result = await app.libraryTool("library_import", { files: [song, fresh, whoosh, outside] });
    expect(result.isError).toBe(false);
    expect(JSON.parse(result.text)).toEqual({
      files: [
        { path: song, media_id: "m-song", name: "Song.wav", bin: "Master/Music", reused: true },
        { path: fresh, media_id: "imported-0", name: "Fresh.wav", bin: "Library/Music", reused: false },
        { path: whoosh, media_id: "imported-0", name: "Whoosh.wav", bin: "Library/Sound effects", reused: false },
      ],
      not_in_library: [outside],
    });
    const imports = calls().filter((call) => call[0] === "AddItemListToMediaPool" || call[0] === "AddSubFolder");
    // The stand-in starts from its state file on every script, so it makes "Library" again for the second bin.
    expect(imports).toEqual([
      ["AddSubFolder", "Master", "Library"],
      ["AddSubFolder", "Library", "Music"],
      ["AddItemListToMediaPool", "Music", [fresh]],
      ["AddSubFolder", "Master", "Library"],
      ["AddSubFolder", "Library", "Sound effects"],
      ["AddItemListToMediaPool", "Sound effects", [whoosh]],
    ]);
    // Changing the project backs it up first, once per turn.
    expect(backups).toEqual([workspaceProject.dir]);

    // A file that's all in the media pool already imports nothing.
    const again = await app.libraryTool("library_import", { files: [song], timeline: "AI - Cut" });
    expect(JSON.parse(again.text)).toEqual({ files: [{ path: song, media_id: "m-song", name: "Song.wav", bin: "Master/Music", reused: true }] });
    endTurn();

    const used = JSON.parse((await app.libraryTool("library_search", { query: "song" })).text) as { results: { used_in?: string[] }[] };
    expect(used.results[0]!.used_in).toEqual(["Doc · Edit", "Doc · AI - Cut"]);
  });
});
