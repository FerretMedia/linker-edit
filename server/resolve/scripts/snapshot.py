# Every timeline of the open project, for the edit history. Read-only.
#
# Field choice follows Vit (github.com/LucasHJin/vit, MIT): cuts, audio, markers and settings, with media
# referenced by media pool item so a timeline can be rebuilt with AppendToTimeline.
#
# ARGS: {"ids": null | [timeline id, ...], "current_only": bool, "budget": seconds}. Scripts may run 60 seconds at most, so this
# stops after `budget` seconds and lists what it didn't get to in "remaining"; Node asks again for those.
import time

started = time.time()
media_cache = {}


def media_of(item):
    mpi = call(item, "GetMediaPoolItem")
    if not mpi:
        return None
    media_id = call(mpi, "GetUniqueId")
    if media_id not in media_cache:
        media_cache[media_id] = {
            "id": media_id,
            "name": call(mpi, "GetName"),
            "path": call(mpi, "GetClipProperty", "File Path") or "",
            "fps": call(mpi, "GetClipProperty", "FPS"),
        }
    return media_cache[media_id]


def item_of(item, track_type):
    source_start, source_end = source_range(item)
    out = {
        "id": call(item, "GetUniqueId"),
        "name": call(item, "GetName"),
        "start": call(item, "GetStart"),
        "end": call(item, "GetEnd"),
        "source_start": source_start,
        "source_end": source_end,
        "media": media_of(item),
        "enabled": call(item, "GetClipEnabled"),
        "properties": plain(call(item, "GetProperty")) or {},
    }
    if track_type == "video":
        out["fusion_comps"] = call(item, "GetFusionCompCount") or 0
    return out


def tracks_of(timeline, track_type):
    tracks = []
    for index in range(1, int(call(timeline, "GetTrackCount", track_type) or 0) + 1):
        tracks.append({
            "index": index,
            "name": call(timeline, "GetTrackName", track_type, index),
            "items": [item_of(i, track_type) for i in (call(timeline, "GetItemListInTrack", track_type, index) or [])],
        })
    return tracks


def timeline_of(timeline):
    markers = plain(call(timeline, "GetMarkers")) or {}
    out = timeline_summary(timeline)
    out["video"] = tracks_of(timeline, "video")
    out["audio"] = tracks_of(timeline, "audio")
    out["markers"] = [dict(frame=int(float(frame)), **info) for frame, info in markers.items()]
    return out


p = need_project()
current = call(p, "GetCurrentTimeline")
timelines = all_timelines()
wanted = ARGS.get("ids")
if ARGS.get("current_only"):
    # The preview of what Resolve shows now needs only that timeline, not the whole project.
    timelines = [current] if current else []
elif wanted is not None:
    timelines = [t for t in timelines if call(t, "GetUniqueId") in wanted]
done = []
remaining = []
for t in timelines:
    # >= so that a budget of 0 reads one timeline per run even where the clock barely moves (Windows).
    if done and time.time() - started >= ARGS.get("budget", 40):
        remaining.append(call(t, "GetUniqueId"))
    else:
        done.append(timeline_of(t))
result = {
    "project": p.GetName(),
    "project_id": call(p, "GetUniqueId"),
    "current_timeline": call(current, "GetName") if current else None,
    "timelines": done,
    "remaining": remaining,
}
