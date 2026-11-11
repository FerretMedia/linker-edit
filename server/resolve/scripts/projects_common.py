# Shared by list_projects.py and open_project.py; Node puts it after common.py.
if not resolve:
    raise Refused("Resolve isn't reachable.")
pm = resolve.GetProjectManager()


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
