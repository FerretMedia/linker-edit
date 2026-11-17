# backup_project: saves the open project and exports it as a .drp file. ARGS: dir (absolute), stamp
p = need_project()
pm = resolve.GetProjectManager()
name = call(p, "GetName") or "Project"
safe = "".join("-" if c in '\\/:*?"<>|' else c for c in name).strip() or "Project"
file = f"{str(ARGS['dir']).rstrip('/')}/{safe} {ARGS['stamp']}.drp"
# Exported from the database, so unsaved changes are saved first.
call(pm, "SaveProject")
if not pm.ExportProject(name, file, False):
    raise Refused(f'Resolve couldn\'t export "{name}" to {file}.')
result = {"project": name, "file": file}
