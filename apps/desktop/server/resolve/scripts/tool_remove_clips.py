# remove_clips: clips off an AI timeline. Their media stays in the media pool. ARGS: timeline, item_ids, ripple
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
items = items_by_id(t, ARGS.get("item_ids"))
if not t.DeleteClips(items, bool(ARGS.get("ripple"))):
    raise Refused("Resolve refused to remove those clips.")
result = {"timeline": call(t, "GetName"), "removed": len(items)}
