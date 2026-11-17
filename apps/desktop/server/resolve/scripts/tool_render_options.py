# render_options: what render_timeline can use. Read-only. ARGS: format (to list its codecs)
p = need_project()
formats = p.GetRenderFormats() or {}
current = p.GetCurrentRenderFormatAndCodec() or {}
fmt = ARGS.get("format") or current.get("format") or "mp4"
result = {
    "presets": p.GetRenderPresetList() or [],
    "formats": formats,
    "codecs": {"format": fmt, "codecs": p.GetRenderCodecs(fmt) or {}},
    "current": current,
}
