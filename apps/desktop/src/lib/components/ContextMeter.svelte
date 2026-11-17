<script lang="ts">
  import type { ContextUsage } from "#shared/protocol.ts";
  import { formatTokens } from "#lib/context.ts";

  let { context, canCompact, onCompact }: { context: ContextUsage; canCompact: boolean; onCompact: () => void } = $props();
  let open = $state(false);
  let root: HTMLDivElement;

  const percent = $derived(
    context.maxTokens ? Math.min(100, Math.round((context.usedTokens / context.maxTokens) * 100)) : null,
  );
  // A static ring: r = 7 gives a circumference of about 44.
  const dash = $derived(percent === null ? 0 : (percent / 100) * 44);

  function closeOnOutside(event: PointerEvent) {
    if (open && !root.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={closeOnOutside} onkeydown={(event) => event.key === "Escape" && (open = false)} />

<div class="relative" bind:this={root}>
  <button
    type="button"
    class="flex h-7 items-center gap-1.5 rounded-md px-2 text-xs text-muted-foreground tabular-nums transition-colors hover:bg-accent hover:text-foreground"
    aria-label="Context: {percent === null ? formatTokens(context.usedTokens) : `${percent}%`} used"
    aria-haspopup="dialog"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    <svg viewBox="0 0 18 18" class="size-3.5 -rotate-90" aria-hidden="true">
      <circle cx="9" cy="9" r="7" fill="none" stroke="currentColor" stroke-opacity="0.25" stroke-width="2.5" />
      <circle
        cx="9"
        cy="9"
        r="7"
        fill="none"
        stroke="currentColor"
        stroke-width="2.5"
        stroke-dasharray="{dash} 44"
        class={percent !== null && percent >= 80 ? "text-warning" : ""}
      />
    </svg>
    {percent === null ? formatTokens(context.usedTokens) : `${percent}%`}
  </button>
  {#if open}
    <div
      class="absolute right-0 bottom-full z-50 mb-1.5 w-64 rounded-lg border bg-card p-3 text-xs shadow-lg shadow-black/10"
      role="dialog"
      aria-label="Context"
    >
      <p class="font-medium text-foreground">
        {context.usedTokens.toLocaleString("en-US")}
        {#if context.maxTokens}of {context.maxTokens.toLocaleString("en-US")}{/if} tokens in context
      </p>
      <p class="mt-1 text-muted-foreground">
        Compacting replaces the chat so far with a summary, so later turns cost less and the chat keeps fitting.
      </p>
      <button
        type="button"
        class="mt-2.5 h-7 w-full rounded-md border text-xs transition-colors enabled:hover:bg-accent disabled:opacity-50"
        disabled={!canCompact}
        onclick={() => {
          open = false;
          onCompact();
        }}
      >
        Compact now
      </button>
    </div>
  {/if}
</div>
