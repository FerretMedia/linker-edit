# get_clip_properties: media pool clips' properties. Read-only. ARGS: media_ids, properties (names)
names = ARGS.get("properties") or []
out = {}
for clip in find_media(ARGS.get("media_ids")):
    props = plain(call(clip, "GetClipProperty")) or {}
    if names:
        props = {k: props.get(k) for k in names}
    out[call(clip, "GetUniqueId")] = {"name": call(clip, "GetName"), "properties": props}
result = out
