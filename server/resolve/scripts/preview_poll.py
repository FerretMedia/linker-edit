# How far the preview render is. ARGS: {"job": job id}. Read-only.
p = need_project()
status = p.GetRenderJobStatus(ARGS["job"]) or {}
result = {
    "rendering": bool(p.IsRenderingInProgress()),
    "percent": int(status.get("CompletionPercentage", 0) or 0),
    "status": status.get("JobStatus"),
}
