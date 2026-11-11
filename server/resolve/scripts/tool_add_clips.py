# add_clips: media pool clips onto an AI timeline, which becomes current. ARGS: timeline, clips
# A clip is {media, source_start, source_end, start, track, type}: the same fields get_timeline and the
# edit history's cuts.json and audio.json use, so a saved timeline's rows can be passed back as they are.
TYPES = {"video": 1, "audio": 2}
p = need_project()
t = find_timeline(ARGS.get("timeline"))
require_ai(t)
clips = ARGS.get("clips") or []
if not clips:
    raise Refused("Pass at least one clip in clips.")
media = dict(zip([str(c.get("media")) for c in clips], find_media([c.get("media") for c in clips])))
infos = []
for clip in clips:
    kind = clip.get("type") or "both"
    if kind not in ("video", "audio", "both"):
        raise Refused(f'A clip\'s type is "video", "audio" or "both", not "{kind}".')
    info = {"mediaPoolItem": media[str(clip.get("media"))]}
    if clip.get("source_start") is not None:
        info["startFrame"] = float(clip["source_start"])
    if clip.get("source_end") is not None:
        info["endFrame"] = float(clip["source_end"])
    if clip.get("start") is not None:
        info["recordFrame"] = float(clip["start"])
    if kind != "both":
        info["mediaType"] = TYPES[kind]
    if clip.get("track") is not None:
        track = int(clip["track"])
        info["trackIndex"] = track
        # Resolve won't add to a track that doesn't exist yet.
        for track_type in (["video", "audio"] if kind == "both" else [kind]):
            while int(t.GetTrackCount(track_type) or 0) < track:
                if not t.AddTrack(track_type):
                    raise Refused(f"Resolve couldn't add {track_type} track {track}.")
    infos.append(info)
p.SetCurrentTimeline(t)
added = p.GetMediaPool().AppendToTimeline(infos) or []
if not added:
    raise Refused(
        "Resolve added none of the clips. Check the media ids and frames, "
        "and that each source range lies inside its clip."
    )
out = []
for item in added:
    track = call(item, "GetTrackTypeAndIndex") or [None, None]
    out.append({
        "id": call(item, "GetUniqueId"),
        "name": call(item, "GetName"),
        "type": track[0],
        "track": track[1],
        "start": call(item, "GetStart"),
        "end": call(item, "GetEnd"),
    })
result = {"timeline": call(t, "GetName"), "added": out}
