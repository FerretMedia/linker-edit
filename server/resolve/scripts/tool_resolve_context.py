# resolve_context: what Resolve has open. Read-only.
p = need_project()
pm = resolve.GetProjectManager()
db = call(pm, "GetCurrentDatabase") or {}
current = p.GetCurrentTimeline()
result = {
    "resolve": f"{resolve.GetProductName()} {resolve.GetVersionString()}",
    "page": resolve.GetCurrentPage(),
    "project": {"name": p.GetName(), "id": call(p, "GetUniqueId")},
    "database": db.get("DbName"),
    "timeline_count": int(p.GetTimelineCount() or 0),
    "current_timeline": timeline_summary(current) if current else None,
    "project_frame_rate": call(p, "GetSetting", "timelineFrameRate"),
}
