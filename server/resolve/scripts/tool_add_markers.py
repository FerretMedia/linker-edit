# add_markers: markers on an AI timeline. ARGS: timeline, markers [{frame, color, name, note, duration}]
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
markers = ARGS.get("markers") or []
if not markers:
    raise Refused("Pass at least one marker.")
added, refused = 0, []
for m in markers:
    ok = call(
        t,
        "AddMarker",
        int(m.get("frame", 0)),
        m.get("color") or "Blue",
        m.get("name") or "",
        m.get("note") or "",
        int(m.get("duration") or 1),
        m.get("custom_data") or "",
    )
    if ok:
        added += 1
    else:
        refused.append(m.get("frame"))
result = {"timeline": call(t, "GetName"), "added": added}
if refused:
    result["refused_frames"] = refused
    result["note"] = "Resolve refuses a marker on a frame that already has one, and colors it doesn't know."
