<script lang="ts">
  import { Film, Plus, X } from "@lucide/svelte";
  import type { TimelineStrip } from "#shared/protocol.ts";
  import { CHANGE_STYLE } from "#lib/labels.ts";

  let { strip, detail }: { strip: TimelineStrip; detail: string | null } = $props();

  const signed = (seconds: number) => `${seconds > 0 ? "+" : "−"}${Math.abs(seconds)}s`;
</script>

<div class="flex flex-col gap-1.5">
  <div class="flex min-w-0 items-baseline gap-1.5 text-xs">
    <span class="truncate font-medium" title={strip.name}>{strip.name}</span>
    {#if strip.state === "new"}
      <span class="shrink-0 rounded bg-success/15 px-1 text-2xs font-medium text-success">new</span>
    {/if}
  </div>
  <div class="flex flex-wrap gap-1.5 p-0.5">
    {#each strip.tiles as tile, index (index)}
      {#if tile.kind === "more"}
        <span
          class="flex size-10 items-center justify-center rounded-md border border-dashed text-2xs text-muted-foreground"
          title="{tile.count} unchanged {tile.count === 1 ? 'clip' : 'clips'}"
        >
          +{tile.count}
        </span>
      {:else}
        <span
          class="relative size-10 shrink-0 overflow-hidden rounded-md bg-card {tile.change
            ? `ring-2 ${CHANGE_STYLE[tile.change].ring}`
            : 'border'}"
          title="{tile.name || 'Untitled'}{tile.change ? ` · ${tile.change}` : ''}{tile.delta
            ? ` ${signed(tile.delta)}`
            : ''}"
        >
          {#if tile.image && tile.media}
            <img
              src="/media/{encodeURIComponent(tile.media)}"
              alt=""
              loading="lazy"
              class="size-full object-cover {tile.change === 'removed' ? 'opacity-35 grayscale' : ''}"
            />
          {:else}
            <span class="flex size-full items-center justify-center {tile.change === 'removed' ? 'opacity-35' : ''}">
              <Film class="size-4 text-muted-foreground" />
            </span>
          {/if}
          {#if tile.change === "removed"}
            <span class="absolute inset-0 flex items-center justify-center text-destructive">
              <X class="size-5" strokeWidth={2.5} />
            </span>
          {:else if tile.change === "added"}
            <span class="absolute top-0.5 right-0.5 flex size-3.5 items-center justify-center rounded-full bg-success text-white">
              <Plus class="size-2.5" strokeWidth={3} />
            </span>
          {/if}
          {#if tile.delta}
            <span class="absolute inset-x-0 bottom-0 bg-black/70 text-center text-[0.5625rem] leading-3.5 font-medium text-white">
              {signed(tile.delta)}
            </span>
          {/if}
        </span>
      {/if}
    {/each}
  </div>
  {#if detail}
    <p class="text-2xs leading-snug text-muted-foreground">{detail}</p>
  {/if}
</div>
