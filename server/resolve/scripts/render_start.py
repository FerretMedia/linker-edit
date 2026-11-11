# Starts rendering one timeline, for the preview panel and the render_timeline tool. Node polls with
# render_poll.py, may stop it with render_stop.py, and always calls render_finish.py with the "restore"
# returned here, which puts Resolve back as it was. Each step is its own script because Resolve's MCP
# stops scripts after 60 seconds.
#
# ARGS: dir (Node made it), name (file name without extension), timeline_id (or null: the current one),
# and optionally preset, format, codec, width, height, max_width, from_frame, to_frame, audio, video.
PRESET = "Linker-Edit restore"

import re

p = need_project()
# First of all: asking for render settings opens the Deliver page.
page = resolve.GetCurrentPage()
previous = p.GetCurrentTimeline()
# By id or exact name; the current one without.
timeline = find_timeline(ARGS.get("timeline_id"))
# Without a name, the file is named after the timeline, minus what file names can't hold.
name = ARGS.get("name") or re.sub(r'[\\/:*?"<>|]+', "-", call(timeline, "GetName") or "render").strip() or "render"

format_codec = p.GetCurrentRenderFormatAndCodec() or {}
p.DeleteRenderPreset(PRESET)
restore = {
    "previous_timeline": call(previous, "GetUniqueId") if previous else None,
    "page": page,
    "format": format_codec.get("format"),
    "codec": format_codec.get("codec"),
    "preset": bool(p.SaveAsNewRenderPreset(PRESET)),
    "job": None,
}


def pick_codec(fmt, wanted):
    codecs = p.GetRenderCodecs(fmt) or {}
    names = list(codecs.values())
    if wanted:
        # By Resolve's code ("H264") or the name its menu shows ("H.264").
        match = codecs.get(wanted) or (wanted if wanted in names else None)
        if not match:
            raise Refused(f'Resolve has no codec "{wanted}" for {fmt}. render_options lists them.')
        return match
    for name in ["H264", *names]:
        if name in names and "264" in name:
            return name
    return names[0] if names else None


def even(value):
    return max(2, int(value) // 2 * 2)


try:
    p.SetCurrentTimeline(timeline)
    if ARGS.get("preset"):
        if not p.LoadRenderPreset(ARGS["preset"]):
            raise Refused(f'Resolve has no render preset "{ARGS["preset"]}". render_options lists them.')
    if ARGS.get("format") or not ARGS.get("preset"):
        fmt = ARGS.get("format") or "mp4"
        codec = pick_codec(fmt, ARGS.get("codec"))
        if not codec or not p.SetCurrentRenderFormatAndCodec(fmt, codec):
            raise Refused(f"Resolve can't render {fmt} here.")
    p.SetCurrentRenderMode(1)
    settings = {"TargetDir": ARGS["dir"], "CustomName": name}
    lo, hi = ARGS.get("from_frame"), ARGS.get("to_frame")
    if lo is None and hi is None:
        settings["SelectAllFrames"] = True
    else:
        settings["SelectAllFrames"] = False
        settings["MarkIn"] = int(lo if lo is not None else timeline.GetStartFrame())
        settings["MarkOut"] = int(hi if hi is not None else timeline.GetEndFrame() - 1)
    if ARGS.get("audio") is not None:
        settings["ExportAudio"] = bool(ARGS["audio"])
    if ARGS.get("video") is not None:
        settings["ExportVideo"] = bool(ARGS["video"])
    if not ARGS.get("preset"):
        settings.setdefault("ExportVideo", True)
        settings.setdefault("ExportAudio", True)
    width = float(ARGS.get("width") or timeline.GetSetting("timelineResolutionWidth") or 1920)
    height = float(ARGS.get("height") or timeline.GetSetting("timelineResolutionHeight") or 1080)
    if ARGS.get("max_width"):
        scale = min(1.0, ARGS["max_width"] / width)
        width, height = width * scale, height * scale
    if ARGS.get("width") or ARGS.get("height") or ARGS.get("max_width") or not ARGS.get("preset"):
        settings["FormatWidth"] = even(width)
        settings["FormatHeight"] = even(height)
    if not p.SetRenderSettings(settings):
        raise Refused("Resolve refused the render settings.")
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
    "file_name": output.get("OutputFilename") or name,
    "restore": restore,
}
