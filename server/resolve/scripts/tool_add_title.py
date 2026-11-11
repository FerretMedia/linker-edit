# add_title: a Fusion title (Text+ by default) with text at a frame of an AI timeline, which becomes current.
# ARGS: timeline, text, frame, title, size, font
p = need_project()
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
text = str(ARGS.get("text") or "")
if not text.strip():
    raise Refused("Pass the title's text.")
p.SetCurrentTimeline(t)
frame = ARGS.get("frame")
if frame is not None and not call(t, "SetCurrentTimecode", frame_timecode(t, frame)):
    raise Refused(f"Resolve couldn't move the playhead to frame {frame}.")
name = ARGS.get("title") or "Text+"
item = call(t, "InsertFusionTitleIntoTimeline", name)
if not item:
    raise Refused(f'Resolve couldn\'t insert the title "{name}". Is a video track there unlocked?')
note = None
comp = call(item, "GetFusionCompByIndex", 1)
tools = list((call(comp, "GetToolList", False, "TextPlus") or {}).values()) if comp else []
if tools:
    tools[0].SetInput("StyledText", text)
    if ARGS.get("size") is not None:
        tools[0].SetInput("Size", float(ARGS["size"]))
    if ARGS.get("font"):
        tools[0].SetInput("Font", str(ARGS["font"]))
else:
    note = "This title has no Text+ tool, so its text wasn't set."
track = call(item, "GetTrackTypeAndIndex") or [None, None]
result = {
    "timeline": call(t, "GetName"),
    "id": call(item, "GetUniqueId"),
    "name": call(item, "GetName"),
    "track": track[1],
    "start": call(item, "GetStart"),
    "end": call(item, "GetEnd"),
}
if note:
    result["note"] = note
