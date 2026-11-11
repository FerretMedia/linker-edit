# organize_media: moves media pool clips into a bin and labels them. Deletes nothing.
# ARGS: media_ids, bin, clip_color, flag, metadata ({field: value})
pool = need_project().GetMediaPool()
clips = find_media(ARGS.get("media_ids"))
refused = []
moved_to = None
if ARGS.get("bin"):
    folder = ensure_folder(ARGS["bin"])
    if pool.MoveClips(clips, folder):
        moved_to = call(folder, "GetName")
    else:
        refused.append("bin")
for clip in clips:
    clip_id = call(clip, "GetUniqueId")
    if ARGS.get("clip_color") and not call(clip, "SetClipColor", ARGS["clip_color"]):
        refused.append(f"{clip_id}: clip_color")
    if ARGS.get("flag") and not call(clip, "AddFlag", ARGS["flag"]):
        refused.append(f"{clip_id}: flag")
    if ARGS.get("metadata") and not call(clip, "SetMetadata", ARGS["metadata"]):
        refused.append(f"{clip_id}: metadata")
result = {"clips": len(clips)}
if moved_to:
    result["moved_to"] = moved_to
if refused:
    result["refused"] = refused
    result["note"] = 'Clip and flag colors are Resolve\'s names ("Orange", "Blue", ...); metadata fields too ("Comments", "Keywords", "Description", "Scene", "Shot", "Take").'
