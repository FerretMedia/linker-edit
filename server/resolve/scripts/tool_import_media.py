# import_media: files into a media pool bin, made when missing. Never removes anything. ARGS: paths, bin
pool = need_project().GetMediaPool()
folder = ensure_folder(ARGS.get("bin") or "AI imports")
previous = pool.GetCurrentFolder()
pool.SetCurrentFolder(folder)
try:
    items = resolve.GetMediaStorage().AddItemListToMediaPool(list(ARGS.get("paths") or [])) or []
finally:
    if previous:
        pool.SetCurrentFolder(previous)
if not items:
    raise Refused("Resolve imported none of the files. Are they media Resolve can read?")
result = {
    "bin": call(folder, "GetName"),
    "imported": [
        {"id": call(i, "GetUniqueId"), "name": call(i, "GetName"), "file_path": call(i, "GetClipProperty", "File Path")}
        for i in items
    ],
}
