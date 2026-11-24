# set_clip_properties: transform, crop, opacity, speed and the like on an AI timeline's clips.
# ARGS: timeline, item_ids, properties ({name: value}), enabled (bool), fade_in, fade_out (frames)
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
items = items_by_id(t, ARGS.get("item_ids"))
properties = ARGS.get("properties") or {}
enabled = ARGS.get("enabled")
fades = {key: int(ARGS[arg]) for key, arg in (("FadeIn", "fade_in"), ("FadeOut", "fade_out")) if ARGS.get(arg) is not None}
if not properties and enabled is None and not fades:
    raise Refused("Pass properties to set, fades, or enabled.")
refused = []
for item in items:
    item_id = call(item, "GetUniqueId")
    for key, value in properties.items():
        if not call(item, "SetProperty", key, value):
            refused.append(f"{item_id}: {key}")
    # Both at once, as Resolve takes them: the one not asked for stays as it is.
    if fades and not call(item, "SetFades", {**(call(item, "GetFades") or {}), **fades}):
        refused.append(f"{item_id}: fades")
    if enabled is not None and not call(item, "SetClipEnabled", bool(enabled)):
        refused.append(f"{item_id}: enabled")
result = {"timeline": call(t, "GetName"), "clips": len(items)}
if refused:
    result["refused"] = refused
    result["note"] = (
        "Resolve refused these. Check the property names and value types with search_scripting_api, "
        "or get_scripting_docs section 'Looking up Timeline item properties'."
    )
