# export_timeline: writes a timeline to an interchange file. Resolve writes it; the project is unchanged.
# ARGS: timeline, format, dir (absolute, made by Node), name (file name without extension), extension
import re

FORMATS = {
    "otio": ("EXPORT_OTIO", "EXPORT_NONE"),
    "edl": ("EXPORT_EDL", "EXPORT_NONE"),
    "fcpxml": ("EXPORT_FCPXML_1_10", "EXPORT_NONE"),
    "fcp7xml": ("EXPORT_FCP_7_XML", "EXPORT_NONE"),
    "aaf": ("EXPORT_AAF", "EXPORT_AAF_NEW"),
    "csv": ("EXPORT_TEXT_CSV", "EXPORT_NONE"),
    "drt": ("EXPORT_DRT", "EXPORT_NONE"),
}
t = find_timeline(ARGS.get("timeline"))
kind, subtype = FORMATS[ARGS["format"]]
stem = re.sub(r'[\\/:*?"<>|]+', "-", ARGS.get("name") or call(t, "GetName") or "timeline").strip() or "timeline"
path = f"{ARGS['dir']}/{stem}.{ARGS['extension']}"
if not t.Export(path, getattr(resolve, kind), getattr(resolve, subtype)):
    raise Refused(f"Resolve couldn't export \"{call(t, 'GetName')}\" as {ARGS['format']}.")
result = {"timeline": call(t, "GetName"), "path": path, "format": ARGS["format"]}
