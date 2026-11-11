# list_media: media pool folders and clips. Read-only. ARGS: bin (folder path), search, limit
import re

KEEP = ["Type", "File Path", "FPS", "Frames", "Duration", "Resolution", "Start TC", "Audio Ch"]
search = str(ARGS.get("search") or "").lower()
wanted_bin = str(ARGS.get("bin") or "").strip("/")
limit = int(ARGS.get("limit") or 200)
folders = media_folders()
clips = []
more = 0
for path, folder in folders:
    if wanted_bin and not (path == wanted_bin or path.startswith(wanted_bin + "/") or path.endswith("/" + wanted_bin)):
        continue
    for clip in call(folder, "GetClipList") or []:
        name = call(clip, "GetName") or ""
        props = call(clip, "GetClipProperty") or {}
        if search and search not in name.lower() and search not in str(props.get("File Path", "")).lower():
            continue
        if len(clips) >= limit:
            more += 1
            continue
        info = {"id": call(clip, "GetUniqueId"), "name": name, "bin": path}
        for key in KEEP:
            value = props.get(key)
            if value not in (None, ""):
                info[re.sub(r"\W+", "_", key.lower())] = value
        clips.append(info)
result = {"bins": [path for path, _ in folders], "clips": clips}
if more:
    result["more"] = f"{more} more clips match. Narrow it down with bin or search, or raise limit."
