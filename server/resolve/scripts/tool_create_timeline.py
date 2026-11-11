# create_timeline: a new, empty "AI - " timeline, made current. ARGS: name, width, height, frame_rate, start_timecode
p = need_project()
pool = p.GetMediaPool()
name = ai_name(ARGS.get("name"))
t = pool.CreateEmptyTimeline(name, ARGS["start_timecode"]) if ARGS.get("start_timecode") else pool.CreateEmptyTimeline(name)
if not t:
    raise Refused(f'Resolve couldn\'t create the timeline "{name}".')
custom = {
    "timelineResolutionWidth": ARGS.get("width"),
    "timelineResolutionHeight": ARGS.get("height"),
    "timelineFrameRate": ARGS.get("frame_rate"),
}
custom = {k: str(v) for k, v in custom.items() if v not in (None, "")}
refused = []
if custom:
    t.SetSetting("useCustomSettings", "1")
    for key, value in custom.items():
        if not t.SetSetting(key, value):
            refused.append(key)
p.SetCurrentTimeline(t)
result = timeline_summary(t)
if refused:
    result["refused_settings"] = refused
