# Stops the preview render and waits for Resolve to finish stopping: it refuses the cleanup until then.
import time

p = need_project()
p.StopRendering()
deadline = time.time() + 30
while p.IsRenderingInProgress() and time.time() < deadline:
    time.sleep(0.25)
result = {"stopped": not p.IsRenderingInProgress()}
