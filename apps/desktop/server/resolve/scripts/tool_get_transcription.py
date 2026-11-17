# get_transcription: the transcript Resolve keeps on a clip. Read-only. ARGS: media_id, words (bool)
p = need_project()
clip = find_media([ARGS.get("media_id")])[0]
status = plain(call(p, "GetTranscribeAudioStatus")) or {}
data = plain(call(clip, "GetTranscription")) or {}
segments = data.get("segments") or []
if not ARGS.get("words"):
    segments = [{k: v for k, v in s.items() if k != "words"} for s in segments]
result = {"language": data.get("language"), "segments": segments}
if status.get("JobStatus") not in (None, "Inactive", "Complete", "Cancelled"):
    result["transcribing"] = status
if not segments:
    result["note"] = "This clip has no transcript in Resolve yet. transcribe_in_resolve makes one; the linker transcribe tool is faster for reading speech."
