# Starts rendering a small MP4 preview of one timeline. Node polls with preview_poll.py, may stop it with
# preview_stop.py, and always calls preview_finish.py with the "restore" returned here, which puts Resolve
# back as it was. Each step is its own script because Resolve's MCP stops scripts after 60 seconds.
#
# ARGS: {"dir": output folder (Node made it), "name": file name without extension, "timeline_id": id or null}
PRESET = "Linker-Edit restore"
MAX_WIDTH = 960

p = need_project()
previous = p.GetCurrentTimeline()
if ARGS.get("timeline_id"):
    timeline = next((t for t in all_timelines() if call(t, "GetUniqueId") == ARGS["timeline_id"]), None)
    if not timeline:
        raise Refused("That timeline no longer exists in Resolve.")
else:
    timeline = previous
    if not timeline:
        raise Refused("Resolve has no timeline open.")

format_codec = p.GetCurrentRenderFormatAndCodec() or {}
p.DeleteRenderPreset(PRESET)
restore = {
    "previous_timeline": call(previous, "GetUniqueId") if previous else None,
    "page": resolve.GetCurrentPage(),
    "format": format_codec.get("format"),
    "codec": format_codec.get("codec"),
    "preset": bool(p.SaveAsNewRenderPreset(PRESET)),
    "job": None,
}


def pick_codec():
    codecs = p.GetRenderCodecs("mp4") or {}
    names = list(codecs.values())
    for name in ["H264", *names]:
        if name in names and "264" in name:
            return name
    return names[0] if names else None


def even(value):
    return max(2, int(value) // 2 * 2)


try:
    p.SetCurrentTimeline(timeline)
    codec = pick_codec()
    if not codec or not p.SetCurrentRenderFormatAndCodec("mp4", codec):
        raise Refused("Resolve can't render MP4 here.")
    p.SetCurrentRenderMode(1)
    width = float(timeline.GetSetting("timelineResolutionWidth") or 1920)
    height = float(timeline.GetSetting("timelineResolutionHeight") or 1080)
    scale = min(1.0, MAX_WIDTH / width)
    if not p.SetRenderSettings({
        "SelectAllFrames": True,
        "TargetDir": ARGS["dir"],
        "CustomName": ARGS["name"],
        "ExportVideo": True,
        "ExportAudio": True,
        "FormatWidth": even(width * scale),
        "FormatHeight": even(height * scale),
    }):
        raise Refused("Resolve refused the preview render settings.")
    job = p.AddRenderJob()
    if not job:
        raise Refused("Resolve couldn't add the render job. Is the Project Manager or a dialog open?")
    restore["job"] = job
    output = next((j for j in p.GetRenderJobList() if j.get("JobId") == job), {})
    if not p.StartRendering([job], False):
        raise Refused("Resolve didn't start rendering.")
except Exception as error:
    # Node only learns the restore state from a result, so put Resolve back here.
    if restore["job"]:
        p.DeleteRenderJob(restore["job"])
    if restore["preset"]:
        p.LoadRenderPreset(PRESET)
        p.DeleteRenderPreset(PRESET)
    if restore["format"] and restore["codec"]:
        p.SetCurrentRenderFormatAndCodec(restore["format"], restore["codec"])
    if previous:
        p.SetCurrentTimeline(previous)
    if restore["page"]:
        resolve.OpenPage(restore["page"])
    raise error

result = {
    "timeline": {"id": timeline.GetUniqueId(), "name": timeline.GetName()},
    "target_dir": output.get("TargetDir") or ARGS["dir"],
    "file_name": output.get("OutputFilename") or f"{ARGS['name']}.mp4",
    "restore": restore,
}
