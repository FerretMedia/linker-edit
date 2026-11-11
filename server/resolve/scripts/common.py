# Shared by every script Linker-Edit runs through Resolve's own MCP server (its run_script tool).
# The server runs each script in a fresh, sandboxed Python with `resolve` and `project` already set,
# and returns what the script puts in `result`. There is no os, sys or open() in there: Node does the
# file work. Linker-Edit puts `ARGS` (the call's input) and this file in front of each script.

AI_PREFIX = "AI - "


class Refused(Exception):
    """A refusal or error meant for the agent or the user; Node shows only its message."""


def call(obj, method, *args):
    """obj.method(*args), or None when the method is missing or raises: Resolve's API varies by version."""
    fn = getattr(obj, method, None)
    if fn is None:
        return None
    try:
        return fn(*args)
    except Exception:
        return None


def plain(value):
    """Only what JSON can carry; Resolve objects become None."""
    if isinstance(value, (str, int, float, bool)) or value is None:
        return value
    if isinstance(value, dict):
        return {str(k): plain(v) for k, v in value.items()}
    if isinstance(value, (list, tuple)):
        return [plain(v) for v in value]
    return None


def need_project():
    if not resolve:
        raise Refused("Resolve isn't reachable.")
    current = resolve.GetProjectManager().GetCurrentProject()
    if not current:
        raise Refused("No project is open in Resolve.")
    return current


def all_timelines():
    p = need_project()
    found = (p.GetTimelineByIndex(i) for i in range(1, int(p.GetTimelineCount() or 0) + 1))
    return [t for t in found if t]


def find_timeline(ref):
    """A timeline by id or exact name; the current one when ref is empty."""
    p = need_project()
    if not ref:
        current = p.GetCurrentTimeline()
        if not current:
            raise Refused("Resolve has no timeline open. Say which timeline with `timeline`.")
        return current
    timelines = all_timelines()
    for t in timelines:
        if call(t, "GetUniqueId") == ref:
            return t
    for t in timelines:
        if call(t, "GetName") == ref:
            return t
    raise Refused(f'There is no timeline "{ref}" in this project. list_timelines shows them.')


def require_ai(timeline):
    """Linker-Edit only changes timelines an agent made: those named "AI - ...". """
    name = call(timeline, "GetName") or ""
    if not name.startswith(AI_PREFIX):
        raise Refused(
            f'"{name}" isn\'t an AI timeline, so Linker-Edit won\'t change it. '
            "Copy it with duplicate_timeline and work on the copy."
        )


def ai_name(name, taken=None):
    """ "Cut" → "AI - Cut", without "/" (Resolve refuses it), made unique among the project's timelines."""
    name = str(name or "").strip()
    if name.startswith(AI_PREFIX):
        name = name[len(AI_PREFIX):]
    name = name.replace("/", "-").strip(" -")
    if not name:
        raise Refused("Give the timeline a name.")
    if taken is None:
        taken = {call(t, "GetName") for t in all_timelines()}
    candidate = AI_PREFIX + name
    number = 2
    while candidate in taken:
        candidate = f"{AI_PREFIX}{name} {number}"
        number += 1
    return candidate


def timeline_summary(t):
    setting = lambda key: call(t, "GetSetting", key)
    return {
        "id": call(t, "GetUniqueId"),
        "name": call(t, "GetName"),
        "frame_rate": setting("timelineFrameRate"),
        "width": setting("timelineResolutionWidth"),
        "height": setting("timelineResolutionHeight"),
        "start_frame": call(t, "GetStartFrame"),
        "end_frame": call(t, "GetEndFrame"),
        "start_timecode": call(t, "GetStartTimecode"),
    }


def track_items(t, track_type):
    """(track index, item) for every item on the timeline's tracks of this type."""
    out = []
    for index in range(1, int(call(t, "GetTrackCount", track_type) or 0) + 1):
        for item in call(t, "GetItemListInTrack", track_type, index) or []:
            out.append((index, item))
    return out


def items_by_id(t, ids):
    wanted = [str(i) for i in ids or []]
    if not wanted:
        raise Refused("Pass the clips' ids in item_ids. get_timeline lists them.")
    found = {}
    for track_type in ("video", "audio"):
        for _, item in track_items(t, track_type):
            item_id = call(item, "GetUniqueId")
            if item_id in wanted:
                found[item_id] = item
    missing = [i for i in wanted if i not in found]
    if missing:
        raise Refused(f"These clips aren't on the timeline: {', '.join(missing)}. get_timeline lists its clips.")
    return [found[i] for i in wanted]


def source_range(item):
    start, end = call(item, "GetSourceStartFrame"), call(item, "GetSourceEndFrame")
    if start is None:
        offset, duration = call(item, "GetLeftOffset"), call(item, "GetDuration")
        if offset is not None and duration is not None:
            start, end = offset, offset + duration
    return start, end


def media_folders():
    """(path, folder) for every media pool folder, the root first; paths read like "Master/Interviews"."""
    root = need_project().GetMediaPool().GetRootFolder()
    out = []

    def walk(folder, path, depth):
        out.append((path, folder))
        if depth >= 20:
            return
        for sub in call(folder, "GetSubFolderList") or []:
            walk(sub, f"{path}/{call(sub, 'GetName')}", depth + 1)

    walk(root, call(root, "GetName") or "Master", 0)
    return out


def find_media(ids):
    """Media pool items by id, in the order asked; refuses when one is missing."""
    wanted = [str(i) for i in ids or []]
    found = {}
    for _, folder in media_folders():
        for clip in call(folder, "GetClipList") or []:
            clip_id = call(clip, "GetUniqueId")
            if clip_id in wanted:
                found[clip_id] = clip
        if len(found) == len(set(wanted)):
            break
    missing = [i for i in wanted if i not in found]
    if missing:
        raise Refused(f"These media ids aren't in the media pool: {', '.join(missing)}. list_media shows them.")
    return [found[i] for i in wanted]
