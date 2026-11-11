# Lists the projects of the database Resolve has open. Other databases can't be listed without switching
# to them, which closes the open project.
projects = []
pm.GotoRootFolder()
walk([], lambda name, path: projects.append({"name": name, "folder": path}))
pm.GotoRootFolder()
result = {
    "database": database(pm.GetCurrentDatabase()),
    "databases": [db for db in (database(db) for db in pm.GetDatabaseList() or []) if db],
    "projects": projects,
}
