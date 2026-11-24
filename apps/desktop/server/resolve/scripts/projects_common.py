# Shared by list_projects.py, open_project.py and create_project.py; Node puts it after common.py.
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


def untouched(p):
    # What Resolve has open behind its Project Manager after starting: new, empty and never saved. Told by
    # its contents and not its name, which may be in the user's language.
    media = call(p, "GetMediaPool")
    root = call(media, "GetRootFolder") if media else None
    if call(p, "GetTimelineCount") != 0 or not root or call(root, "GetClipList") or call(root, "GetSubFolderList"):
        return False
    return p.GetName() not in (pm.GetProjectListInCurrentFolder() or [])


def saved_somewhere(name):
    pm.GotoRootFolder()
    found = walk([], lambda n, p: n == name)
    pm.GotoRootFolder()
    return found


def check_leaving(current):
    """Refuses to leave a project with work that would be lost. With Resolve's UI up, loading a project over one with
    unsaved changes raises a dialog no script can dismiss, so callers save it. The empty "Untitled Project" Resolve
    starts with isn't saved: that would add it to the user's database."""
    if call(current, "IsRenderingInProgress"):
        raise Refused(f"Resolve is rendering in \"{current.GetName()}\". Wait until it's done.")
    # Saving it would ask the user for a name, or add an "Untitled Project" to their database. What a
    # Blackmagic Cloud library lists isn't known, so there it's left alone.
    if local_library() and not saved_somewhere(current.GetName()):
        raise Refused(f"\"{current.GetName()}\" has work in it and was never saved. Save it in Resolve first.")


def local_library():
    return ((pm.GetCurrentDatabase() or {}).get("DbType") or "").lower() in ("disk", "postgresql")


def opened(p, folder):
    return {"project": {"name": p.GetName(), "id": p.GetUniqueId()}, "database": database(pm.GetCurrentDatabase()), "folder": folder}
