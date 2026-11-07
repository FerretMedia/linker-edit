"""Print every timeline of Resolve's current project as one JSON line. Read-only.

Field choice follows Vit (github.com/LucasHJin/vit, MIT): cuts, audio, markers and settings,
with media referenced by media pool item so a timeline can be rebuilt with AppendToTimeline.
"""
import json
import sys

try:
    import DaVinciResolveScript as dvr
except Exception as error:
    print(json.dumps({"error": f"Resolve scripting module failed to load: {error}"}))
    sys.exit()


def call(obj, method, *args):
    fn = getattr(obj, method, None)
    if fn is None:
        return None
    try:
        return fn(*args)
    except Exception:
        return None


def plain(value):
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    if isinstance(value, dict):
        return {str(k): plain(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [plain(v) for v in value]
    return None


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
        }
    return media_cache[media_id]


def item_of(item, track_type):
    out = {
        "id": call(item, "GetUniqueId"),
        "name": call(item, "GetName"),
        "start": call(item, "GetStart"),
        "end": call(item, "GetEnd"),
        "source_start": call(item, "GetSourceStartFrame"),
        "source_end": call(item, "GetSourceEndFrame"),
        "media": media_of(item),
        "enabled": call(item, "GetClipEnabled"),
        "properties": plain(call(item, "GetProperty")) or {},
    }
    if out["source_start"] is None:
        offset = call(item, "GetLeftOffset")
        duration = call(item, "GetDuration")
        if offset is not None and duration is not None:
            out["source_start"], out["source_end"] = offset, offset + duration
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
    setting = lambda key: call(timeline, "GetSetting", key)
    markers = plain(call(timeline, "GetMarkers")) or {}
    return {
        "id": call(timeline, "GetUniqueId"),
        "name": call(timeline, "GetName"),
        "frame_rate": setting("timelineFrameRate"),
        "width": setting("timelineResolutionWidth"),
        "height": setting("timelineResolutionHeight"),
        "start_frame": call(timeline, "GetStartFrame"),
        "end_frame": call(timeline, "GetEndFrame"),
        "start_timecode": call(timeline, "GetStartTimecode"),
        "video": tracks_of(timeline, "video"),
        "audio": tracks_of(timeline, "audio"),
        "markers": [dict(frame=int(float(frame)), **info) for frame, info in markers.items()],
    }


resolve = dvr.scriptapp("Resolve")
if not resolve:
    print(json.dumps({"error": "Resolve is not reachable."}))
    sys.exit()
project = resolve.GetProjectManager().GetCurrentProject()
if not project:
    print(json.dumps({"error": "No project is open in Resolve."}))
    sys.exit()
current = call(project, "GetCurrentTimeline")
timelines = [project.GetTimelineByIndex(i) for i in range(1, int(project.GetTimelineCount() or 0) + 1)]
print(json.dumps({
    "project": project.GetName(),
    "current_timeline": call(current, "GetName") if current else None,
    "timelines": [timeline_of(t) for t in timelines if t],
}))
