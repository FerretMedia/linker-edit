# duplicate_timeline: an "AI - " copy of a timeline, made current. ARGS: timeline, name
p = need_project()
source = find_timeline(ARGS.get("timeline"))
source_name = call(source, "GetName") or "Timeline"
name = ai_name(ARGS.get("name") or f"{source_name} copy")
copy = source.DuplicateTimeline(name)
if not copy:
    raise Refused(f'Resolve couldn\'t duplicate "{source_name}".')
p.SetCurrentTimeline(copy)
result = timeline_summary(copy)
result["copied_from"] = {"id": call(source, "GetUniqueId"), "name": source_name}
