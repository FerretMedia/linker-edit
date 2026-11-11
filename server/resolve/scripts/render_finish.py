# Puts Resolve's timeline and render settings back as they were before a render. ARGS: {"restore": what render_start.py returned}.
PRESET = "Linker-Edit restore"
restore = ARGS["restore"]
p = need_project()
if restore.get("job"):
    p.DeleteRenderJob(restore["job"])
if restore.get("preset"):
    p.LoadRenderPreset(PRESET)
    p.DeleteRenderPreset(PRESET)
if restore.get("format") and restore.get("codec"):
    p.SetCurrentRenderFormatAndCodec(restore["format"], restore["codec"])
if restore.get("previous_timeline"):
    previous = next((t for t in all_timelines() if call(t, "GetUniqueId") == restore["previous_timeline"]), None)
    if previous:
        p.SetCurrentTimeline(previous)
# The page goes back in render_page.py, run separately: Resolve ignores a page switch made in this script.
result = True
