# set_current_timeline: shows a timeline in Resolve. ARGS: timeline
p = need_project()
t = find_timeline(ARGS.get("timeline"))
if not p.SetCurrentTimeline(t):
    raise Refused("Resolve refused to switch timelines.")
result = timeline_summary(t)
