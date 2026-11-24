# Saves the open project, then opens another. ARGS: {kind, name, projectId, database, folder, onlyIfIdle}.
# A cloud project is never opened here, only recognised when you've opened it in Resolve yourself.
#
# The open project is saved first, as Resolve's own Project Manager would offer; see check_leaving.


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


def open_project(req):
    current = pm.GetCurrentProject()
    if current and req.get("projectId") and current.GetUniqueId() == req["projectId"]:
        return opened(current, None)
    if req["kind"] == "cloud":
        # Blackmagic Cloud's terms (31.5) don't allow scripts to use the service, so Linker-Edit never calls
        # Resolve's cloud project API: you open the project in Resolve yourself (see AGENTS.md).
        if current and not req.get("projectId") and current.GetName() == req["name"]:
            return opened(current, None)
        raise Refused(
            f"Open \"{req['name']}\" yourself in Resolve's Project Manager, under Blackmagic Cloud. "
            "Linker-Edit doesn't open cloud projects for you: Blackmagic Cloud's terms don't allow scripts to use it."
        )
    idle = not current or untouched(current)
    if not idle and req.get("onlyIfIdle"):
        raise Refused(
            f"Resolve has \"{current.GetName()}\" open, and only the user switches projects. Ask them to open "
            f"\"{req['name']}\" with Open in Resolve in Linker-Edit."
        )
    if not idle:
        check_leaving(current)
    previous_name = current.GetName() if current else None
    previous_db = database(pm.GetCurrentDatabase())
    if not idle:
        pm.SaveProject()

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
