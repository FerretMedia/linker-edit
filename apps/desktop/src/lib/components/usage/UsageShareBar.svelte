<script lang="ts">
  // T3 Code's apps/web/src/components/usage/UsageShareBar.tsx (MIT, see NOTICE).
  import type { Snippet } from "svelte";
  import { formatPercent, type ShareSegment } from "#lib/usage.ts";

  let { label, segments, format, aside }: { label: string; segments: ShareSegment[]; format: (value: number) => string; aside?: Snippet } = $props();

  const visible = $derived(segments.filter((segment) => segment.value > 0));
  const total = $derived(visible.reduce((sum, segment) => sum + segment.value, 0));
</script>

{#if total > 0}
  <div class="flex min-w-0 flex-col gap-2.5">
    <div class="flex items-baseline justify-between gap-3">
      <h3 class="text-sm font-medium text-foreground">{label}</h3>
      {@render aside?.()}
    </div>
    <!-- The legend below carries every figure; the bar's tooltips are pointer-only. -->
    <div aria-hidden="true" class="flex h-2 gap-0.5">
      {#each visible as segment (segment.label)}
        <div
          class="h-full min-w-1 rounded-xs first:rounded-l-full last:rounded-r-full"
          style:flex="{segment.value} 1 0"
          style:background-color={segment.color}
          title="{segment.label} · {format(segment.value)} · {formatPercent(segment.value / total)}"
        ></div>
      {/each}
    </div>
    <ul aria-label={label} class="flex flex-wrap gap-x-4 gap-y-1 text-xs">
      {#each visible as segment (segment.label)}
        <li class="flex items-center gap-1.5">
          <span aria-hidden="true" class="size-2 rounded-xs" style:background-color={segment.color}></span>
          <span class="text-muted-foreground">{segment.label}</span>
          <span class="text-foreground tabular-nums">{format(segment.value)}</span>
          <span class="sr-only">, {formatPercent(segment.value / total)}</span>
        </li>
      {/each}
    </ul>
  </div>
{/if}
