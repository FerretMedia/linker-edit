<script lang="ts" module>
  import type { Component } from "svelte";

  export interface MenuItem {
    label: string;
    Icon: Component<{ class?: string }>;
    onSelect: () => void;
    danger?: boolean;
  }
</script>

<script lang="ts">
  import { Ellipsis } from "@lucide/svelte";

  let { items, label }: { items: MenuItem[]; label: string } = $props();
  let open = $state(false);
  let root: HTMLDivElement;

  function closeOnOutside(event: PointerEvent) {
    if (open && !root.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={closeOnOutside} onkeydown={(event) => event.key === "Escape" && (open = false)} />

<div class="relative" bind:this={root}>
  <button
    type="button"
    class="flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    aria-label={label}
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={(event) => {
      event.stopPropagation();
      open = !open;
    }}
  >
    <Ellipsis class="size-3.5" />
  </button>
  {#if open}
    <div class="absolute top-full right-0 z-50 mt-1 w-52 rounded-lg border bg-card p-1 shadow-lg shadow-black/10" role="menu">
      {#each items as item (item.label)}
        <button
          type="button"
          role="menuitem"
          class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors hover:bg-accent {item.danger
            ? 'text-destructive'
            : ''}"
          onclick={(event) => {
            event.stopPropagation();
            open = false;
            item.onSelect();
          }}
        >
          <item.Icon class="size-3.5 shrink-0" />
          {item.label}
        </button>
      {/each}
    </div>
  {/if}
</div>
