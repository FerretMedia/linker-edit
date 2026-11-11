# list_timelines: every timeline in the project. Read-only.
p = need_project()
current = p.GetCurrentTimeline()
current_id = call(current, "GetUniqueId") if current else None
out = []
for t in all_timelines():
    info = timeline_summary(t)
    info["current"] = info["id"] == current_id
    info["ai"] = (info["name"] or "").startswith(AI_PREFIX)
    info["video_tracks"] = int(call(t, "GetTrackCount", "video") or 0)
    info["audio_tracks"] = int(call(t, "GetTrackCount", "audio") or 0)
    out.append(info)
result = out
