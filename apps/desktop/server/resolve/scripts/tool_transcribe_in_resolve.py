# transcribe_in_resolve: starts Resolve's own transcription of clips; the transcript is stored on each clip.
# ARGS: media_ids, speakers (bool)
started, refused = [], []
for clip in find_media(ARGS.get("media_ids")):
    if call(clip, "TranscribeAudio", bool(ARGS.get("speakers"))):
        started.append(call(clip, "GetUniqueId"))
    else:
        refused.append(call(clip, "GetUniqueId"))
result = {"started": started, "status": plain(call(need_project(), "GetTranscribeAudioStatus"))}
if refused:
    result["refused"] = refused
result["note"] = "Resolve transcribes in the background. get_transcription shows the progress and, once done, the text."
