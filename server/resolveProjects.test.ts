import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { ResolveError } from "./resolve/connection.ts";
import { fakeConnection, hasPython, pythonConnection } from "./resolve/testing.ts";
import { createResolveProjects } from "./resolveProjects.ts";

// Stands in for Resolve's scripting module: a Project Manager over folder trees from a JSON file,
// logging every call that changes something.
const FAKE_MODULE = String.raw`
import json, os
STATE = json.load(open(os.environ["FAKE_RESOLVE"], encoding="utf8"))
def log(*args):
    with open(os.environ["FAKE_RESOLVE"] + ".log", "a", encoding="utf8") as f:
        f.write(json.dumps(args) + "\n")

class Project:
    def __init__(self, name, id):
        self.name, self.id = name, id
    def GetName(self): return self.name
    def GetUniqueId(self): return self.id

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
  const projects = createResolveProjects(pythonConnection(FAKE_MODULE, { FAKE_RESOLVE: stateFile }));
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

  it("opens a cloud project with this computer's media folder, and says what's missing", async () => {
    const { projects, calls } = fakeResolve({ cloud: { "Client X": "cx-id" } });
    const link = { kind: "cloud", name: "Client X", projectId: null } as const;
    await expect(projects.open(link, {})).rejects.toThrow("needs a media folder");
    expect((await projects.open(link, { mediaPath: "D:\\Cloud", syncMode: "proxy_and_orig" })).project).toEqual({ name: "Client X", id: "cx-id" });
    expect(calls()).toEqual([
      ["SaveProject", "Promo"],
      ["LoadCloudProject", { name: "Client X", media: "D:\\Cloud", sync: 2 }],
    ]);
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

describe("opening a project that takes longer than a script may run", () => {
  it("waits for Resolve to show it open", async () => {
    let polls = 0;
    const { connection } = fakeConnection({
      script: (name) => {
        if (name === "open_project") throw new ResolveError("timeout", "Script timed out after 60s.");
        polls += 1;
        return { project: polls < 3 ? "Promo" : "Client X", projectId: polls < 3 ? "promo-id" : "cx-id", database: null };
      },
    });
    const opened = await createResolveProjects(connection, 1).open({ kind: "cloud", name: "Client X", projectId: "cx-id" }, { mediaPath: "D:/Cloud" });
    expect(opened).toEqual({ project: { name: "Client X", id: "cx-id" }, database: null, folder: null });
    expect(polls).toBe(3);
  });
});
