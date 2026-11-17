# Saves the open project, then opens another. ARGS: {kind, name, projectId, database, folder, mediaPath, syncMode}.
#
# With Resolve's UI up, loading a project over one with unsaved changes raises a dialog no script can
# dismiss, so the open project is saved first, as Resolve's own Project Manager would offer.


def db_info(db):
    info = {"DbType": db["type"], "DbName": db["name"]}
    if db.get("host"):
        info["IpAddress"] = db["host"]
    return info


def load_by_name(name, hint):
    # The folder it was last found in first, then a search of every folder.
    pm.GotoRootFolder()
    if hint and all(pm.OpenFolder(f) for f in hint):
        loaded = pm.LoadProject(name)
        if loaded:
            pm.GotoRootFolder()
            return loaded, hint
    pm.GotoRootFolder()
    found = []

    def visit(n, p):
        if n == name:
            found.append(p)
        return bool(found)

    walk([], visit)
    pm.GotoRootFolder()
    if found and all(pm.OpenFolder(f) for f in found[0]):
        loaded = pm.LoadProject(name)
        pm.GotoRootFolder()
        if loaded:
            return loaded, found[0]
    pm.GotoRootFolder()
    return None, None


def opened(p, folder):
    return {"project": {"name": p.GetName(), "id": p.GetUniqueId()}, "database": database(pm.GetCurrentDatabase()), "folder": folder}


def open_project(req):
    current = pm.GetCurrentProject()
    if current and req.get("projectId") and current.GetUniqueId() == req["projectId"]:
        return opened(current, None)
    if req["kind"] == "cloud" and not req.get("mediaPath"):
        raise Refused("Resolve needs a media folder on this computer to open a cloud project. Set one in the project's settings.")
    previous_name = current.GetName() if current else None
    previous_db = database(pm.GetCurrentDatabase())
    if current:
        pm.SaveProject()

    loaded = None
    folder = None
    if req["kind"] == "cloud":
        settings = {resolve.CLOUD_SETTING_PROJECT_NAME: req["name"], resolve.CLOUD_SETTING_PROJECT_MEDIA_PATH: req["mediaPath"]}
        mode = getattr(resolve, "CLOUD_SYNC_" + (req.get("syncMode") or "proxy_only").upper(), None)
        if mode is not None:
            settings[resolve.CLOUD_SETTING_SYNC_MODE] = mode
        loaded = pm.LoadCloudProject(settings)
    else:
        db = req.get("database")
        switched = bool(db and previous_db != db)
        if switched and not pm.SetCurrentDatabase(db_info(db)):
            raise Refused(f"Resolve couldn't switch to the database \"{db['name']}\".")
        loaded, folder = load_by_name(req["name"], req.get("folder") or [])
        # Switching databases closed the user's project: rather than leave nothing open, go back to it.
        if not loaded and switched and previous_db and pm.SetCurrentDatabase(db_info(previous_db)) and previous_name:
            load_by_name(previous_name, [])

    if not loaded:
        raise Refused(f"Resolve couldn't find or open the project \"{req['name']}\".")
    return opened(loaded, folder)


result = open_project(ARGS)
