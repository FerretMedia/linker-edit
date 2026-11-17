# get_timeline: a timeline's clips and markers. Read-only.
# ARGS: timeline, track_type ("video" | "audio"), track, from_frame, to_frame, properties (bool)
t = find_timeline(ARGS.get("timeline"))
lo, hi = ARGS.get("from_frame"), ARGS.get("to_frame")
media = {}
out = timeline_summary(t)
for track_type in ("video", "audio"):
    tracks = []
    if ARGS.get("track_type") in (None, track_type):
        for index in range(1, int(call(t, "GetTrackCount", track_type) or 0) + 1):
            if ARGS.get("track") is not None and int(ARGS["track"]) != index:
                continue
            items = []
            for item in call(t, "GetItemListInTrack", track_type, index) or []:
                start, end = call(item, "GetStart"), call(item, "GetEnd")
                if (hi is not None and start is not None and start > hi) or (lo is not None and end is not None and end < lo):
                    continue
                source_start, source_end = source_range(item)
                mpi = call(item, "GetMediaPoolItem")
                media_id = call(mpi, "GetUniqueId") if mpi else None
                if media_id and media_id not in media:
                    media[media_id] = {"name": call(mpi, "GetName"), "path": call(mpi, "GetClipProperty", "File Path") or "", "fps": call(mpi, "GetClipProperty", "FPS")}
                clip = {"id": call(item, "GetUniqueId"), "name": call(item, "GetName"), "start": start, "end": end, "source_start": source_start, "source_end": source_end, "media": media_id}
                if call(item, "GetClipEnabled") is False:
                    clip["enabled"] = False
                if ARGS.get("properties"):
                    clip["properties"] = plain(call(item, "GetProperty")) or {}
                items.append(clip)
            tracks.append({"track": index, "name": call(t, "GetTrackName", track_type, index), "items": items})
    out[track_type] = tracks
markers = plain(call(t, "GetMarkers")) or {}
out["markers"] = [dict(frame=int(float(frame)), **info) for frame, info in sorted(markers.items(), key=lambda m: float(m[0]))]
out["media"] = media
result = out
