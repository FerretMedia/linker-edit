# Resolve's state for the status pill and the project check before each turn. Read-only.


def activity(p, t, page):
    # Differs when the user did something: Resolve doesn't say whether a project was changed. Clips are
    # compared one by one only on timelines short enough to read on every check.
    if not t:
        return json.dumps([page, call(p, "GetTimelineCount")])
    tracks = [call(t, "GetItemListInTrack", kind, i) or [] for kind in ("video", "audio", "subtitle") for i in range(1, int(call(t, "GetTrackCount", kind) or 0) + 1)]
    clips = sum(len(items) for items in tracks)
    edges = [[[call(item, "GetStart"), call(item, "GetEnd")] for item in items] for items in tracks] if clips <= 300 else []
    return json.dumps([
        page,
        call(p, "GetTimelineCount"),
        call(t, "GetUniqueId"),
        call(t, "GetCurrentTimecode"),
        call(t, "GetEndFrame"),
        [len(items) for items in tracks],
        sum((i + 1) * (a or 0) + (i + 7) * (b or 0) for i, (a, b) in enumerate(edge for items in edges for edge in items)),
        len(call(t, "GetMarkers") or {}),
    ])


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
        out["activity"] = activity(p, t, out["page"])
    result = out
