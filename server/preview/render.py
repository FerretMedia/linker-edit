"""Render a small MP4 preview of one timeline, then put Resolve back as it was.

Usage: render.py <output dir> <file name> [timeline id]. Without an id it renders the current timeline.
Prints one JSON object per line: {"event": "start" | "progress" | "done" | "error", ...}.
A line "stop" on stdin cancels the render.
"""
import json
import os
import sys
import threading
import time

PRESET = "Linker-Edit restore"
MAX_WIDTH = 960


def emit(event, **fields):
    print(json.dumps({"event": event, **fields}), flush=True)


try:
    import DaVinciResolveScript as dvr
except Exception as error:
    emit("error", message=f"Resolve scripting module failed to load: {error}")
    sys.exit()

out_dir, file_name = sys.argv[1], sys.argv[2]
timeline_id = sys.argv[3] if len(sys.argv) > 3 else ""

stop = threading.Event()


def watch_stdin():
    for line in sys.stdin:
        if line.strip() == "stop":
            stop.set()
            return


threading.Thread(target=watch_stdin, daemon=True).start()

resolve = dvr.scriptapp("Resolve")
project = resolve and resolve.GetProjectManager().GetCurrentProject()
if not project:
    emit("error", message="Resolve has no project open.")
    sys.exit()

timelines = [project.GetTimelineByIndex(i) for i in range(1, project.GetTimelineCount() + 1)]
previous = project.GetCurrentTimeline()
if timeline_id:
    timeline = next((t for t in timelines if t and t.GetUniqueId() == timeline_id), None)
else:
    timeline = previous
if not timeline:
    emit("error", message="That timeline no longer exists in Resolve." if timeline_id else "Resolve has no timeline open.")
    sys.exit()


def pick_codec():
    codecs = project.GetRenderCodecs("mp4") or {}
    names = list(codecs.values())
    for name in ["H264", *names]:
        if name in names and "264" in name:
            return name
    return names[0] if names else None


def even(value):
    return max(2, int(value) // 2 * 2)


page = resolve.GetCurrentPage()
result = ("error", {"message": "The preview render stopped without a result."})
format_codec = project.GetCurrentRenderFormatAndCodec() or {}
project.DeleteRenderPreset(PRESET)
saved_preset = project.SaveAsNewRenderPreset(PRESET)
job = None
try:
    project.SetCurrentTimeline(timeline)
    codec = pick_codec()
    if not codec or not project.SetCurrentRenderFormatAndCodec("mp4", codec):
        raise RuntimeError("Resolve can't render MP4 here.")
    project.SetCurrentRenderMode(1)

    width = float(timeline.GetSetting("timelineResolutionWidth") or 1920)
    height = float(timeline.GetSetting("timelineResolutionHeight") or 1080)
    scale = min(1.0, MAX_WIDTH / width)
    os.makedirs(out_dir, exist_ok=True)
    if not project.SetRenderSettings(
        {
            "SelectAllFrames": True,
            "TargetDir": out_dir,
            "CustomName": file_name,
            "ExportVideo": True,
            "ExportAudio": True,
            "FormatWidth": even(width * scale),
            "FormatHeight": even(height * scale),
        }
    ):
        raise RuntimeError("Resolve refused the preview render settings.")

    job = project.AddRenderJob()
    if not job:
        raise RuntimeError("Resolve couldn't add the render job. Is the Project Manager or a dialog open?")
    output = next((j for j in project.GetRenderJobList() if j.get("JobId") == job), {})
    emit("start", timeline={"id": timeline.GetUniqueId(), "name": timeline.GetName()})
    if not project.StartRendering([job], False):
        raise RuntimeError("Resolve didn't start rendering.")

    last = -1
    while project.IsRenderingInProgress():
        if stop.is_set():
            project.StopRendering()
            # Resolve takes a moment to stop and refuses the cleanup below until it has.
            deadline = time.time() + 30
            while project.IsRenderingInProgress() and time.time() < deadline:
                time.sleep(0.25)
            break
        percent = int((project.GetRenderJobStatus(job) or {}).get("CompletionPercentage", 0))
        if percent != last:
            emit("progress", percent=percent)
            last = percent
        time.sleep(0.5)

    status = project.GetRenderJobStatus(job) or {}
    path = os.path.join(output.get("TargetDir", out_dir), output.get("OutputFilename", f"{file_name}.mp4"))
    if stop.is_set():
        if os.path.exists(path):
            os.remove(path)
        result = ("error", {"message": "Preview stopped.", "stopped": True})
    elif status.get("JobStatus") != "Complete" or not os.path.exists(path):
        result = ("error", {"message": f"Resolve's render ended with status {status.get('JobStatus', 'unknown')}."})
    else:
        result = ("done", {"file": path})
except Exception as error:
    result = ("error", {"message": str(error)})
finally:
    if job:
        project.DeleteRenderJob(job)
    if saved_preset:
        project.LoadRenderPreset(PRESET)
        project.DeleteRenderPreset(PRESET)
    if format_codec.get("format") and format_codec.get("codec"):
        project.SetCurrentRenderFormatAndCodec(format_codec["format"], format_codec["codec"])
    if previous:
        project.SetCurrentTimeline(previous)
    if page:
        resolve.OpenPage(page)

# Only now, with Resolve put back, may the app let the agent at it again.
emit(result[0], **result[1])
