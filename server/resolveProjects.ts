import { spawn } from "node:child_process";
import type { LocalLink, ResolveDatabase, ResolveLibrary, ResolveLink } from "../shared/protocol.ts";
import type { ResolveMcpConfig } from "./config.ts";

const PRELUDE = `
import json, sys
try:
    import DaVinciResolveScript as d
except Exception as e:
    print(json.dumps({"error": f"Resolve scripting module failed to load: {e}"})); sys.exit()
r = d.scriptapp("Resolve")
if not r:
    print(json.dumps({"error": "Resolve is not reachable."})); sys.exit()
pm = r.GetProjectManager()

def database(db):
    if not db or not db.get("DbName"):
        return None
    out = {"type": db.get("DbType") or "", "name": db["DbName"]}
    if db.get("IpAddress"):
        out["host"] = db["IpAddress"]
    return out

def folder_name(folder):
    return folder if isinstance(folder, str) else folder.GetName()

def walk(path, visit, depth=0):
    # Moves the Project Manager's folder, never the open project; callers go back to the root after.
    for name in pm.GetProjectListInCurrentFolder() or []:
        if visit(name, path):
            return True
    if depth >= 12:
        return False
    for folder in pm.GetFolderListInCurrentFolder() or []:
        name = folder_name(folder)
        if pm.OpenFolder(name):
            found = walk(path + [name], visit, depth + 1)
            pm.GotoParentFolder()
            if found:
                return True
    return False
`;

// Lists the projects of the database Resolve has open. Other databases can't be listed without
// switching to them, which closes the open project.
const LIST = `${PRELUDE}
projects = []
pm.GotoRootFolder()
walk([], lambda name, path: projects.append({"name": name, "folder": path}))
pm.GotoRootFolder()
print(json.dumps({
    "database": database(pm.GetCurrentDatabase()),
    "databases": [db for db in (database(db) for db in pm.GetDatabaseList() or []) if db],
    "projects": projects,
}))
`;

// With Resolve's UI up, loading a project over one with unsaved changes raises a dialog no script
// can dismiss, so the open project is saved first, as Resolve's own Project Manager would offer.
const OPEN = `${PRELUDE}
def db_info(db):
    info = {"DbType": db["type"], "DbName": db["name"]}
    if db.get("host"):
        info["IpAddress"] = db["host"]
    return info

def load_by_name(name, hint):
    # The folder it was last found in first, then a search of every folder.
    pm.GotoRootFolder()
    if hint and all(pm.OpenFolder(f) for f in hint):
        project = pm.LoadProject(name)
        if project:
            pm.GotoRootFolder()
            return project, hint
    pm.GotoRootFolder()
    found = []
    def visit(n, p):
        if n == name:
            found.append(p)
        return bool(found)
    walk([], visit)
    pm.GotoRootFolder()
    if found and all(pm.OpenFolder(f) for f in found[0]):
        project = pm.LoadProject(name)
        pm.GotoRootFolder()
        if project:
            return project, found[0]
    pm.GotoRootFolder()
    return None, None

req = json.loads(sys.stdin.read())
current = pm.GetCurrentProject()
if current and req.get("projectId") and current.GetUniqueId() == req["projectId"]:
    print(json.dumps({"project": {"name": current.GetName(), "id": current.GetUniqueId()}, "database": database(pm.GetCurrentDatabase()), "folder": None}))
    sys.exit()
if req["kind"] == "cloud" and not req.get("mediaPath"):
    print(json.dumps({"error": "Resolve needs a media folder on this computer to open a cloud project. Set one in the project's settings."})); sys.exit()
previous_name = current.GetName() if current else None
previous_db = database(pm.GetCurrentDatabase())
if current:
    pm.SaveProject()

project = None
folder = None
if req["kind"] == "cloud":
    settings = {r.CLOUD_SETTING_PROJECT_NAME: req["name"], r.CLOUD_SETTING_PROJECT_MEDIA_PATH: req["mediaPath"]}
    mode = getattr(r, "CLOUD_SYNC_" + (req.get("syncMode") or "proxy_only").upper(), None)
    if mode is not None:
        settings[r.CLOUD_SETTING_SYNC_MODE] = mode
    project = pm.LoadCloudProject(settings)
else:
    db = req.get("database")
    switched = bool(db and previous_db != db)
    if switched and not pm.SetCurrentDatabase(db_info(db)):
        print(json.dumps({"error": f"Resolve couldn't switch to the database \\"{db['name']}\\"."})); sys.exit()
    project, folder = load_by_name(req["name"], req.get("folder") or [])
    # Switching databases closed the user's project: rather than leave nothing open, go back to it.
    if not project and switched and previous_db and pm.SetCurrentDatabase(db_info(previous_db)) and previous_name:
        load_by_name(previous_name, [])

if not project:
    print(json.dumps({"error": f"Resolve couldn't find or open the project \\"{req['name']}\\"."})); sys.exit()
print(json.dumps({"project": {"name": project.GetName(), "id": project.GetUniqueId()}, "database": database(pm.GetCurrentDatabase()), "folder": folder}))
`;

export interface OpenedProject {
  project: { name: string; id: string | null };
  database: ResolveDatabase | null;
  /** The Project Manager folders it was found in; null when that isn't known. */
  folder: string[] | null;
}

export interface ResolveProjects {
  library(): Promise<ResolveLibrary>;
  /** Saves Resolve's open project, then opens this one. */
  open(link: ResolveLink, local: LocalLink): Promise<OpenedProject>;
}

function runPython<T>(mcp: ResolveMcpConfig, script: string, input: string, timeout: number): Promise<T> {
  return new Promise((resolve, reject) => {
    const child = spawn(mcp.python, ["-c", script], { windowsHide: true, env: { ...process.env, ...mcp.env } });
    let stdout = "";
    let stderr = "";
    const timer = setTimeout(() => {
      child.kill();
      reject(new Error("Resolve didn't answer in time. Is a dialog open in Resolve?"));
    }, timeout);
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.on("error", (error) => {
      clearTimeout(timer);
      reject(error);
    });
    child.on("close", () => {
      clearTimeout(timer);
      // Resolve's scripting module prints warnings of its own; the result is the last line.
      try {
        const result = JSON.parse(stdout.trim().split("\n").pop() ?? "") as T | { error: string };
        if (result && typeof result === "object" && "error" in result) reject(new Error(result.error));
        else resolve(result as T);
      } catch {
        reject(new Error(stderr.trim().split("\n").pop() || "The Resolve script printed no result."));
      }
    });
    child.stdin.end(input);
  });
}

export function createResolveProjects(mcp: ResolveMcpConfig): ResolveProjects {
  return {
    library: () => runPython<ResolveLibrary>(mcp, LIST, "", 60_000),
    // Opening a cloud project can download it first.
    open: (link, local) =>
      runPython<OpenedProject>(
        mcp,
        OPEN,
        JSON.stringify({
          kind: link.kind,
          name: link.name,
          projectId: link.projectId,
          database: local.database ?? null,
          folder: local.folder ?? [],
          mediaPath: local.mediaPath ?? null,
          syncMode: local.syncMode ?? null,
        }),
        5 * 60_000,
      ),
  };
}
