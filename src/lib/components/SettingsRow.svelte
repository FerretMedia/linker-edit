<script lang="ts">
  import type { Snippet } from "svelte";

  // One setting: what it is on the left, its control on the right, stacked when the window is narrow.
  let {
    label,
    description,
    detail,
    children,
  }: { label: string; description?: string; detail?: Snippet; children?: Snippet } = $props();
</script>

<div class="flex flex-col gap-3 p-4 sm:flex-row sm:items-center sm:gap-6">
  <div class="min-w-0 flex-1">
    <p class="text-sm">{label}</p>
    {#if description}
      <p class="mt-0.5 text-xs text-muted-foreground">{description}</p>
    {/if}
    {@render detail?.()}
  </div>
  {#if children}
    <div class="flex shrink-0 items-center gap-2">
      {@render children()}
    </div>
  {/if}
</div>
