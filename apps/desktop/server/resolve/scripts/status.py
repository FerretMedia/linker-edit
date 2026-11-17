# Resolve's state for the status pill and the project check before each turn. Read-only.
if not resolve:
    result = {"state": "unreachable"}
else:
    pm = resolve.GetProjectManager()
    out = {
        "state": "connected",
        "product": resolve.GetProductName(),
        "version": resolve.GetVersionString(),
        "project": None,
        "projectId": None,
        "database": None,
        "timeline": None,
        # None while the Project Manager or a modal dialog is up.
        "page": resolve.GetCurrentPage(),
    }
    db = call(pm, "GetCurrentDatabase") or {}
    if db.get("DbName"):
        out["database"] = {"type": db.get("DbType") or "", "name": db["DbName"]}
        if db.get("IpAddress"):
            out["database"]["host"] = db["IpAddress"]
    p = pm.GetCurrentProject()
    if p:
        out["project"] = p.GetName()
        out["projectId"] = call(p, "GetUniqueId") or None
        t = p.GetCurrentTimeline()
        out["timeline"] = t.GetName() if t else None
    result = out
