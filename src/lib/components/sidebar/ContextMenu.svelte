<script lang="ts" module>
  import type { Component } from "svelte";

  /** A row of T3 Code's thread and project menus. */
  export interface ContextMenuItem {
    id: string;
    label: string;
    Icon?: Component<{ class?: string }>;
    /** Shown on the right, like a snooze preset's wake time. */
    detail?: string;
    destructive?: boolean;
    disabled?: boolean;
    separatorBefore?: boolean;
    children?: ContextMenuItem[];
    onSelect?: () => void;
  }
</script>

<script lang="ts">
  import { ChevronRight } from "@lucide/svelte";
  import { tick } from "svelte";
  import Self from "./ContextMenu.svelte";

  let {
    x,
    y,
    items,
    onClose,
    submenu = false,
    align = "start",
  }: {
    x: number;
    y: number;
    items: ContextMenuItem[];
    onClose: () => void;
    submenu?: boolean;
    /** "end" puts the menu's right edge at x, as for a button's menu. */
    align?: "start" | "end";
  } = $props();

  let menu: HTMLDivElement | undefined = $state();
  let position = $state({ left: 0, top: 0 });
  let openChild = $state<{ id: string; x: number; y: number } | null>(null);

  $effect(() => {
    void x;
    void y;
    position = { left: x, top: y };
    // Kept on screen: flipped left or up when it would run off the window.
    void tick().then(() => {
      if (!menu) return;
      const rect = menu.getBoundingClientRect();
      const start = align === "end" ? x - rect.width : x;
      position = {
        left:
          start + rect.width > innerWidth - 8
            ? Math.max(8, submenu ? x - rect.width - menu.parentElement!.offsetWidth : x - rect.width)
            : Math.max(8, start),
        top: y + rect.height > innerHeight - 8 ? Math.max(8, innerHeight - rect.height - 8) : y,
      };
      if (!submenu) menu.querySelector<HTMLElement>("[role=menuitem]:not([aria-disabled=true])")?.focus();
    });
  });

  function choose(item: ContextMenuItem, event: MouseEvent) {
    if (item.disabled) return;
    if (item.children) {
      const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
      openChild = { id: item.id, x: rect.right + 2, y: rect.top - 5 };
      return;
    }
    onClose();
    item.onSelect?.();
  }

  function hover(item: ContextMenuItem, event: PointerEvent) {
    if (!item.children || item.disabled) {
      openChild = null;
      return;
    }
    const rect = (event.currentTarget as HTMLElement).getBoundingClientRect();
    openChild = { id: item.id, x: rect.right + 2, y: rect.top - 5 };
  }

  function onKeydown(event: KeyboardEvent) {
    if (submenu) return;
    if (event.key === "Escape") {
      event.preventDefault();
      onClose();
      return;
    }
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    event.preventDefault();
    const rows = [...document.querySelectorAll<HTMLElement>("[data-context-menu] [role=menuitem]:not([aria-disabled=true])")];
    const index = rows.indexOf(document.activeElement as HTMLElement);
    rows[(index + (event.key === "ArrowDown" ? 1 : -1) + rows.length) % rows.length]?.focus();
  }

  function onPointerDown(event: PointerEvent) {
    if (submenu) return;
    if (!(event.target as Element).closest?.("[data-context-menu]")) onClose();
  }
</script>

<svelte:window onkeydown={onKeydown} onpointerdown={onPointerDown} onblur={() => !submenu && onClose()} onresize={() => !submenu && onClose()} />

<div
  bind:this={menu}
  data-context-menu
  role="menu"
  tabindex="-1"
  class="fixed z-[130] flex max-h-[calc(100vh-1rem)] max-w-[calc(100vw-2rem)] min-w-[min(10rem,calc(100vw-2rem))] flex-col overflow-y-auto rounded-lg border border-foreground/10 bg-popover p-1 shadow-[0_16px_40px_-18px_rgb(0_0_0/55%)] outline-none dark:shadow-[0_18px_44px_-18px_rgb(0_0_0/80%)]"
  style="left: {position.left}px; top: {position.top}px"
  oncontextmenu={(event) => event.preventDefault()}
>
  {#each items as item (item.id)}
    {#if item.separatorBefore}
      <div role="separator" class="mx-2 my-1 h-px bg-border"></div>
    {/if}
    <button
      type="button"
      role="menuitem"
      aria-disabled={item.disabled || undefined}
      aria-haspopup={item.children ? "menu" : undefined}
      aria-expanded={item.children ? openChild?.id === item.id : undefined}
      class="flex min-h-7 w-full cursor-pointer items-center gap-2 rounded-sm px-2 py-1 text-left text-sm whitespace-nowrap outline-none select-none hover:bg-accent focus-visible:bg-accent aria-disabled:pointer-events-none aria-disabled:cursor-not-allowed aria-disabled:opacity-64 aria-expanded:bg-accent [&>svg]:-mx-0.5 [&>svg]:size-4 [&>svg]:shrink-0 {item.destructive
        ? 'text-destructive [&>svg]:text-current'
        : 'text-foreground [&>svg:first-child]:text-muted-foreground [&>svg:first-child]:opacity-80'}"
      onclick={(event) => choose(item, event)}
      onpointerenter={(event) => hover(item, event)}
      onkeydown={(event) => {
        if (event.key === "ArrowRight" && item.children) choose(item, event as unknown as MouseEvent);
      }}
    >
      {#if item.Icon}
        <item.Icon />
      {/if}
      <span class="min-w-0 flex-1">{item.label}</span>
      {#if item.detail}
        <span class="ms-auto shrink-0 ps-4 text-secondary-label tabular-nums">{item.detail}</span>
      {/if}
      {#if item.children}
        <ChevronRight class="-me-0.5 ms-auto opacity-80" />
      {/if}
    </button>
    {#if item.children && openChild?.id === item.id}
      <Self x={openChild.x} y={openChild.y} items={item.children} {onClose} submenu />
    {/if}
  {/each}
</div>
