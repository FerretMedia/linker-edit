# set_tracks: names, turns on or off, and locks an AI timeline's tracks, adding missing ones.
# ARGS: timeline, tracks [{type, index, name, enabled, locked}]
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
tracks = ARGS.get("tracks") or []
if not tracks:
    raise Refused("Pass the tracks to change.")
refused = []
for track in tracks:
    kind, index = track.get("type"), int(track.get("index") or 0)
    if kind not in ("video", "audio", "subtitle") or index < 1:
        raise Refused('A track needs type "video", "audio" or "subtitle" and an index from 1.')
    while int(t.GetTrackCount(kind) or 0) < index:
        if not t.AddTrack(kind):
            raise Refused(f"Resolve couldn't add {kind} track {index}.")
    label = f"{kind} {index}"
    if track.get("name") is not None and not call(t, "SetTrackName", kind, index, str(track["name"])):
        refused.append(f"{label}: name")
    if track.get("enabled") is not None and not call(t, "SetTrackEnable", kind, index, bool(track["enabled"])):
        refused.append(f"{label}: enabled")
    if track.get("locked") is not None and not call(t, "SetTrackLock", kind, index, bool(track["locked"])):
        refused.append(f"{label}: locked")
result = {
    "timeline": call(t, "GetName"),
    "tracks": {kind: [call(t, "GetTrackName", kind, i) for i in range(1, int(t.GetTrackCount(kind) or 0) + 1)] for kind in ("video", "audio")},
}
if refused:
    result["refused"] = refused
