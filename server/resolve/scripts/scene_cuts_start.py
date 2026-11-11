# detect_scene_cuts: has Resolve cut an AI timeline's clips at scene changes. Node polls scene_cuts_status.py.
# ARGS: timeline
p = need_project()
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
p.SetCurrentTimeline(t)
before = len(track_items(t, "video"))
if not t.DetectSceneCuts():
    raise Refused("Resolve couldn't detect scene cuts on this timeline.")
result = {"timeline_id": call(t, "GetUniqueId"), "before": before, "status": plain(call(p, "GetDetectSceneCutsStatus")) or {}}
