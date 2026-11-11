# import_media: files into a media pool bin, made when missing. Never removes anything. ARGS: paths, bin
p = need_project()
pool = p.GetMediaPool()
folder = pool.GetRootFolder()
for name in [part.strip() for part in str(ARGS.get("bin") or "AI imports").split("/") if part.strip()]:
    sub = next((f for f in call(folder, "GetSubFolderList") or [] if call(f, "GetName") == name), None)
    folder = sub or pool.AddSubFolder(folder, name)
    if not folder:
        raise Refused(f'Resolve couldn\'t make the bin "{name}".')
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
