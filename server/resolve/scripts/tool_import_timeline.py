# import_timeline: a new AI timeline from an OTIO, EDL, FCPXML, FCP 7 XML or AAF file, made current.
# ARGS: path (absolute, checked by Node), name, import_media (bool)
p = need_project()
pool = p.GetMediaPool()
name = ai_name(ARGS.get("name"))
options = {"timelineName": name, "importSourceClips": bool(ARGS.get("import_media"))}
if not options["importSourceClips"]:
    # Link to clips already in the media pool rather than importing the files again.
    options["sourceClipsFolders"] = [folder for _, folder in media_folders()]
t = pool.ImportTimelineFromFile(ARGS["path"], options)
if not t:
    raise Refused(
        "Resolve couldn't import that file as a timeline. Check its format, and that its media is in the media pool "
        "(or pass import_media to import the files it names)."
    )
if (call(t, "GetName") or "") != name and not call(t, "SetName", name):
    raise Refused(f'Resolve imported the timeline as "{call(t, "GetName")}" and won\'t rename it to "{name}".')
p.SetCurrentTimeline(t)
result = timeline_summary(t)
result["items"] = {kind: len(track_items(t, kind)) for kind in ("video", "audio")}
