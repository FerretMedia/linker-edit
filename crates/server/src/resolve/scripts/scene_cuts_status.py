# How far scene cut detection is, and the timeline's video clips. Read-only. ARGS: timeline
p = need_project()
t = find_timeline(ARGS.get("timeline"))
result = {"status": plain(call(p, "GetDetectSceneCutsStatus")) or {}, "clips": len(track_items(t, "video")), "name": call(t, "GetName")}
