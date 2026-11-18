# remove_markers: markers off an AI timeline, by frame or by color. ARGS: timeline, frames, color
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
frames, color = ARGS.get("frames") or [], ARGS.get("color")
if not frames and not color:
    raise Refused('Pass the markers\' frames, or a color ("All" for every marker).')
removed, refused = 0, []
for frame in frames:
    if call(t, "DeleteMarkerAtFrame", int(frame)):
        removed += 1
    else:
        refused.append(frame)
if color:
    before = len(call(t, "GetMarkers") or {})
    call(t, "DeleteMarkersByColor", color)
    removed += before - len(call(t, "GetMarkers") or {})
result = {"timeline": call(t, "GetName"), "removed": removed}
if refused:
    result["no_marker_at"] = refused
