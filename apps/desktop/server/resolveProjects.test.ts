import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it, vi } from "vitest";
import { ResolveError } from "./resolve/connection.ts";
import { fakeConnection, hasPython, pythonConnection, RESOLVE_IS_UP } from "./resolve/testing.ts";
import { createResolveProjects } from "./resolveProjects.ts";

// Stands in for Resolve's scripting module: a Project Manager over folder trees from a JSON file,
// logging every call that changes something.
const FAKE_MODULE = String.raw`
import json, os
STATE = json.load(open(os.environ["FAKE_RESOLVE"], encoding="utf8"))
def log(*args):
    with open(os.environ["FAKE_RESOLVE"] + ".log", "a", encoding="utf8") as f:
        f.write(json.dumps(args) + "\n")

class Pool:
    def GetRootFolder(self): return self
    def GetClipList(self): return []
    def GetSubFolderList(self): return []

class Project:
    def __init__(self, name, id):
        self.name, self.id = name, id
    def GetName(self): return self.name
    def GetUniqueId(self): return self.id
    def GetTimelineCount(self): return 0 if self.id == "untitled-id" else 1
    def GetCurrentTimeline(self): return None
    def IsRenderingInProgress(self): return STATE.get("rendering", False)
    def GetMediaPool(self): return Pool()

class PM:
    def __init__(self):
        self.path, self.db = [], STATE["database"]
        self.current = Project(*STATE["current"]) if STATE.get("current") else None
    def tree(self):
        node = STATE["trees"][self.db["DbName"]]
        for name in self.path:
            node = node["folders"][name]
        return node
    def GetCurrentDatabase(self): return self.db
    def GetDatabaseList(self): return STATE["databaseList"]
    def SetCurrentDatabase(self, info):
        log("SetCurrentDatabase", info)
        if info["DbName"] not in STATE["trees"]:
            return False
        self.db, self.current, self.path = info, None, []
        return True
    def GotoRootFolder(self):
        self.path = []
        return True
    def GotoParentFolder(self):
        self.path = self.path[:-1]
        return True
    def OpenFolder(self, name):
        if name not in self.tree().get("folders", {}):
            return False
        self.path = self.path + [name]
        return True
    def GetProjectListInCurrentFolder(self): return list(self.tree().get("projects", {}))
    def GetFolderListInCurrentFolder(self): return list(self.tree().get("folders", {}))
    def GetCurrentProject(self): return self.current
    def SaveProject(self):
        log("SaveProject", self.current.GetName())
        return True
    def LoadProject(self, name):
        projects = self.tree().get("projects", {})
        if name not in projects:
            return None
        log("LoadProject", self.path, name)
        self.current = Project(name, projects[name])
        return self.current
    def CreateProject(self, name):
        log("CreateProject", self.path, name)
        if "/" in name:
            return None
        self.current = Project(name, "made-id")
        return self.current
    def LoadCloudProject(self, settings):
        log("LoadCloudProject", settings)
        cloud = STATE.get("cloud", {})
        name = settings.get("name")
        if name not in cloud:
            return None
        self.current = Project(name, cloud[name])
        return self.current

PM_INSTANCE = PM()

class Resolve:
    CLOUD_SETTING_PROJECT_NAME = "name"
    CLOUD_SETTING_PROJECT_MEDIA_PATH = "media"
    CLOUD_SETTING_SYNC_MODE = "sync"
    CLOUD_SYNC_NONE = 0
    CLOUD_SYNC_PROXY_ONLY = 1
    CLOUD_SYNC_PROXY_AND_ORIG = 2
    def GetProjectManager(self): return PM_INSTANCE
    def GetProductName(self): return "DaVinci Resolve Studio"
    def GetVersionString(self): return "21.1.2"
    def GetCurrentPage(self): return None

def scriptapp(name): return Resolve()
`;

const LOCAL = { DbType: "Disk", DbName: "Local Database" };
const TEAM = { DbType: "PostgreSQL", DbName: "Team", IpAddress: "10.0.0.5" };

function fakeResolve(state: Record<string, unknown>) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-fake-resolve-"));
  const stateFile = path.join(dir, "state.json");
  fs.writeFileSync(
    stateFile,
    JSON.stringify({
      database: LOCAL,
      databaseList: [LOCAL, TEAM],
      current: ["Promo", "promo-id"],
      trees: {
        "Local Database": {
          projects: { Promo: "promo-id" },
          folders: { Clients: { folders: { "2026": { projects: { Interview: "int-id" } } } } },
        },
        Team: { projects: { Doc: "doc-id" } },
      },
      ...state,
    }),
  );
  const projects = createResolveProjects(pythonConnection(FAKE_MODULE, { FAKE_RESOLVE: stateFile }), 1, RESOLVE_IS_UP);
  const calls = () =>
    fs.existsSync(`${stateFile}.log`)
      ? fs
          .readFileSync(`${stateFile}.log`, "utf8")
          .trim()
          .split("\n")
          .map((line) => JSON.parse(line) as unknown[])
      : [];
  return { projects, calls };
}

describe.skipIf(!hasPython)("Resolve projects", () => {
  it("lists the open database's projects with their folders, and the other databases", async () => {
    const { projects, calls } = fakeResolve({});
    expect(await projects.library()).toEqual({
      database: { type: "Disk", name: "Local Database" },
      databases: [
        { type: "Disk", name: "Local Database" },
        { type: "PostgreSQL", name: "Team", host: "10.0.0.5" },
      ],
      projects: [
        { name: "Promo", folder: [] },
        { name: "Interview", folder: ["Clients", "2026"] },
      ],
    });
    expect(calls()).toEqual([]);
  });

  it("saves the open project, then finds and opens another one by searching its folders", async () => {
    const { projects, calls } = fakeResolve({});
    expect(await projects.open({ kind: "local", name: "Interview", projectId: null }, { folder: ["Old"] })).toEqual({
      project: { name: "Interview", id: "int-id" },
      database: { type: "Disk", name: "Local Database" },
      folder: ["Clients", "2026"],
    });
    expect(calls()).toEqual([
      ["SaveProject", "Promo"],
      ["LoadProject", ["Clients", "2026"], "Interview"],
    ]);
  });

  it("switches to the project's network database first", async () => {
    const { projects, calls } = fakeResolve({});
    const opened = await projects.open(
      { kind: "network", name: "Doc", projectId: "doc-id" },
      { database: { type: "PostgreSQL", name: "Team", host: "10.0.0.5" } },
    );
    expect(opened.project).toEqual({ name: "Doc", id: "doc-id" });
    expect(calls()).toEqual([["SaveProject", "Promo"], ["SetCurrentDatabase", TEAM], ["LoadProject", [], "Doc"]]);
  });

  it("does nothing when the project is already open", async () => {
    const { projects, calls } = fakeResolve({});
    expect((await projects.open({ kind: "local", name: "Promo", projectId: "promo-id" }, {})).project).toEqual({ name: "Promo", id: "promo-id" });
    expect(calls()).toEqual([]);
  });

  it("never opens a Blackmagic Cloud project itself, only finds it once you've opened it in Resolve", async () => {
    const link = { kind: "cloud", name: "Client X", projectId: null } as const;
    const elsewhere = fakeResolve({ cloud: { "Client X": "cx-id" } });
    await expect(elsewhere.projects.open(link, {})).rejects.toThrow("Open \"Client X\" yourself in Resolve's Project Manager");
    await expect(elsewhere.projects.open({ ...link, projectId: "cx-id" }, {})).rejects.toThrow("terms don't allow scripts");
    expect(elsewhere.calls()).toEqual([]);

    const open = fakeResolve({ current: ["Client X", "cx-id"] });
    expect((await open.projects.open(link, {})).project).toEqual({ name: "Client X", id: "cx-id" });
    expect((await open.projects.open({ ...link, projectId: "cx-id" }, {})).project).toEqual({ name: "Client X", id: "cx-id" });
    expect(open.calls()).toEqual([]);
  });

  it("opens it for an agent only when no other project is open, without saving Resolve's empty untitled one", async () => {
    const link = { kind: "local", name: "Interview", projectId: "int-id" } as const;
    const idle = fakeResolve({ current: ["Unbenannt", "untitled-id"] });
    expect((await idle.projects.open(link, {}, { onlyIfIdle: true })).project).toEqual({ name: "Interview", id: "int-id" });
    expect(idle.calls()).toEqual([["LoadProject", ["Clients", "2026"], "Interview"]]);

    const working = fakeResolve({});
    await expect(working.projects.open(link, {}, { onlyIfIdle: true })).rejects.toThrow('Resolve has "Promo" open');
    expect(working.calls()).toEqual([]);
    const empty = fakeResolve({ current: ["Promo", "untitled-id"] });
    await expect(empty.projects.open(link, {}, { onlyIfIdle: true })).rejects.toThrow('Resolve has "Promo" open');
  });

  it("never leaves a project that is rendering, or has work in it that was never saved", async () => {
    const link = { kind: "local", name: "Interview", projectId: "int-id" } as const;
    const rendering = fakeResolve({ rendering: true });
    await expect(rendering.projects.open(link, {})).rejects.toThrow('Resolve is rendering in "Promo"');
    const unsaved = fakeResolve({ current: ["Untitled Project", "new-id"] });
    await expect(unsaved.projects.open(link, {})).rejects.toThrow("was never saved");
    expect([...rendering.calls(), ...unsaved.calls()]).toEqual([]);
  });

  it("makes a new project in the open database's root, after saving the one that is open", async () => {
    const { projects, calls } = fakeResolve({});
    expect(await projects.create(" Roastery ")).toEqual({
      project: { name: "Roastery", id: "made-id" },
      database: { type: "Disk", name: "Local Database" },
      folder: [],
    });
    expect(calls()).toEqual([["SaveProject", "Promo"], ["CreateProject", [], "Roastery"], ["SaveProject", "Roastery"]]);

    const idle = fakeResolve({ current: ["Unbenannt", "untitled-id"] });
    await idle.projects.create("Roastery");
    expect(idle.calls()).toEqual([["CreateProject", [], "Roastery"], ["SaveProject", "Roastery"]]);
  });

  it("saves an empty project that is kept in a folder before it makes or opens another", async () => {
    const state = { current: ["Interview", "untitled-id"] };
    const made = fakeResolve(state);
    await made.projects.create("Roastery");
    expect(made.calls()[0]).toEqual(["SaveProject", "Interview"]);
    const opened = fakeResolve(state);
    await opened.projects.open({ kind: "local", name: "Promo", projectId: "promo-id" }, {});
    expect(opened.calls()[0]).toEqual(["SaveProject", "Interview"]);
  });

  it("makes no project over unsaved work or a render, with a taken name, or outside a local or network database", async () => {
    const refused = async (state: Record<string, unknown>, name: string, reason: string) => {
      const { projects, calls } = fakeResolve(state);
      await expect(projects.create(name)).rejects.toThrow(reason);
      expect(calls()).toEqual([]);
    };
    await refused({ rendering: true }, "Roastery", 'Resolve is rendering in "Promo"');
    await refused({ current: ["Untitled Project", "new-id"] }, "Roastery", "was never saved");
    await refused({}, "Interview", 'already has a project named "Interview"');
    await refused({ current: ["Client X", "cx-id"], database: { DbType: "Cloud", DbName: "Local Database" } }, "Roastery", "doesn't have a local or network database open");
    await expect(fakeResolve({}).projects.create("a/b")).rejects.toThrow("couldn't make the project");
  });

  it("says when Resolve can't find the project, back in the project it had open", async () => {
    const { projects, calls } = fakeResolve({});
    await expect(
      projects.open({ kind: "network", name: "Gone", projectId: null }, { database: { type: "PostgreSQL", name: "Team", host: "10.0.0.5" } }),
    ).rejects.toThrow('Resolve couldn\'t find or open the project "Gone".');
    expect(calls()).toEqual([
      ["SaveProject", "Promo"],
      ["SetCurrentDatabase", TEAM],
      ["SetCurrentDatabase", LOCAL],
      ["LoadProject", [], "Promo"],
    ]);
  });
});

it("waits for a new project that takes longer to make than a script may run", async () => {
  const { connection } = fakeConnection({
    script: (name) => {
      if (name === "create_project") throw new ResolveError("timeout", "Script timed out after 60s.");
      return { state: "connected", project: "Roastery", projectId: "made-id", database: null };
    },
  });
  expect(await createResolveProjects(connection, 1, RESOLVE_IS_UP).create("Roastery")).toMatchObject({ project: { name: "Roastery", id: "made-id" } });
});

it("doesn't let an agent leave what a Resolve it just started has open", async () => {
  vi.useFakeTimers();
  try {
    const { connection, scripts } = fakeConnection({
      script: (name) => (name === "status" ? { state: "connected" } : { project: { name: "Interview", id: "int-id" }, database: null, folder: null }),
    });
    let launched = false;
    const projects = createResolveProjects(connection, 1, {
      processes: async () => ({ resolve: launched, scriptServer: launched }),
      launch: async () => void (launched = true),
    });
    const opening = projects.open({ kind: "local", name: "Interview", projectId: "int-id" }, {}, { onlyIfIdle: true });
    await vi.advanceTimersByTimeAsync(10_000);
    await opening;
    expect(launched).toBe(true);
    expect(scripts.at(-1)).toMatchObject({ name: "open_project", args: { onlyIfIdle: true } });
  } finally {
    vi.useRealTimers();
  }
});

describe("opening a project that takes longer than a script may run", () => {
  it("waits for Resolve to show it open", async () => {
    let polls = 0;
    let opening = false;
    const { connection } = fakeConnection({
      script: (name) => {
        if (name === "open_project") {
          opening = true;
          throw new ResolveError("timeout", "Script timed out after 60s.");
        }
        if (opening) polls += 1;
        return { state: "connected", project: polls < 3 ? "Promo" : "Client X", projectId: polls < 3 ? "promo-id" : "cx-id", database: null };
      },
    });
    const opened = await createResolveProjects(connection, 1, RESOLVE_IS_UP).open({ kind: "local", name: "Client X", projectId: "cx-id" }, {});
    expect(opened).toEqual({ project: { name: "Client X", id: "cx-id" }, database: null, folder: null });
    expect(polls).toBe(3);
  });
});
