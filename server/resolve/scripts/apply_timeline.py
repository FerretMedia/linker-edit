# apply_timeline: the user approved an AI timeline, so it becomes the main one. Nothing is deleted or
# overwritten: the main timeline is renamed "<name> (before AI <stamp>)" and the AI timeline takes its
# name. Only the user starts this, from Linker-Edit; agents have no tool for it. ARGS: timeline, main, stamp
p = need_project()
if not ARGS.get("timeline") or not ARGS.get("main"):
    raise Refused("Say which AI timeline replaces which timeline.")
ai = find_timeline(ARGS["timeline"])
require_ai(ai)
main = find_timeline(ARGS["main"])
if call(main, "GetUniqueId") == call(ai, "GetUniqueId"):
    raise Refused("That's the same timeline.")
main_name = call(main, "GetName") or ""
ai_timeline_name = call(ai, "GetName") or ""
if main_name.startswith(AI_PREFIX):
    raise Refused(f'"{main_name}" is an AI timeline itself, so there is no main timeline to replace.')

taken = {call(t, "GetName") for t in all_timelines()}
previous = f"{main_name} (before AI {ARGS['stamp']})"
number = 2
while previous in taken:
    previous = f"{main_name} (before AI {ARGS['stamp']}) {number}"
    number += 1

if not main.SetName(previous):
    raise Refused(f'Resolve refused to rename "{main_name}".')
if not ai.SetName(main_name):
    # Put the main timeline's name back, so nothing is left half done.
    main.SetName(main_name)
    raise Refused(f'Resolve refused to rename "{ai_timeline_name}" to "{main_name}".')
p.SetCurrentTimeline(ai)
result = {"main": timeline_summary(ai), "previous": timeline_summary(main), "was": ai_timeline_name}
