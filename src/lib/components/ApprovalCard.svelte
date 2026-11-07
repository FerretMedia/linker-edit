<script lang="ts">
  import { ShieldAlert } from "@lucide/svelte";
  import type { ApprovalRequest } from "#shared/protocol.ts";
  import { formatJson, isResolveTool, toolLabel } from "#lib/labels.ts";

  let { request, onAnswer }: { request: ApprovalRequest; onAnswer: (allow: boolean) => void } = $props();
</script>

<div class="rounded-2xl border border-warning/30 bg-card px-4 py-3 shadow-composer">
  <div class="flex items-center gap-2 text-sm">
    <ShieldAlert class="size-4 shrink-0 text-warning" />
    <span class="min-w-0 flex-1 truncate">
      <span class="font-medium">Allow {isResolveTool(request.tool) ? "Resolve" : "tool"}:</span>
      <span class="font-mono text-xs text-foreground/80">{toolLabel(request.tool, request.input)}</span>
    </span>
  </div>
  <pre
    class="mt-2 ms-6 max-h-40 overflow-auto font-mono text-xs leading-5 whitespace-pre-wrap text-muted-foreground select-text">{formatJson(
      request.input,
    )}</pre>
  <div class="mt-3 flex justify-end gap-2">
    <button
      type="button"
      class="h-7 rounded-md border px-3 text-xs font-medium transition-colors hover:bg-accent"
      onclick={() => onAnswer(false)}
    >
      Deny
    </button>
    <button
      type="button"
      class="h-7 rounded-md bg-primary px-3 text-xs font-medium text-primary-foreground transition-opacity hover:opacity-90"
      onclick={() => onAnswer(true)}
    >
      Allow
    </button>
  </div>
</div>
