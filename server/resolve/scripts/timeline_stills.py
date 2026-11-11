# Exports stills of timeline frames for view_frames, then puts the timeline, playhead and page back.
# Reads only. ARGS: timeline, frames (timeline frames), dir (Node made it)
p = need_project()
t = find_timeline(ARGS.get("timeline"))
previous = p.GetCurrentTimeline()
page = resolve.GetCurrentPage()
p.SetCurrentTimeline(t)
playhead = call(t, "GetCurrentTimecode")
out = []
try:
    for n, frame in enumerate(ARGS.get("frames") or []):
        path = f"{ARGS['dir']}/frame-{n}-{int(frame)}.png"
        moved = call(t, "SetCurrentTimecode", frame_timecode(t, frame))
        out.append({"frame": int(frame), "file": path, "ok": bool(moved and call(p, "ExportCurrentFrameAsStill", path))})
finally:
    if playhead:
        call(t, "SetCurrentTimecode", playhead)
    if previous and call(previous, "GetUniqueId") != call(t, "GetUniqueId"):
        p.SetCurrentTimeline(previous)
    if page and resolve.GetCurrentPage() != page:
        resolve.OpenPage(page)
result = {"timeline": call(t, "GetName"), "stills": out}
