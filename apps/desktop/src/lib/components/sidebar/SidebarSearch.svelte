<script lang="ts">
  // T3 Code's sidebar thread header (apps/web/src/components/sidebar/SidebarThreadHeader.tsx; MIT, see NOTICE):
  // the search box, then the project scope (Add project is in it) and New thread.
  import type { Snippet } from "svelte";
  import Search from "@lucide/svelte/icons/search";
  import SquarePen from "@lucide/svelte/icons/square-pen";
  import X from "@lucide/svelte/icons/x";

  let {
    query = $bindable(""),
    input = $bindable(),
    hasProjects,
    resultCount,
    activeResult,
    onKeydown,
    projectScope,
    onNewThread,
  }: {
    query?: string;
    input?: HTMLInputElement;
    hasProjects: boolean;
    resultCount: number;
    activeResult: number;
    onKeydown: (event: KeyboardEvent) => void;
    /** T3 Code's Projects button; the sidebar leaves it out until there are projects. */
    projectScope: Snippet;
    onNewThread: () => void;
  } = $props();

  const searching = $derived(query.trim().length > 0);
  const resultsVisible = $derived(searching && resultCount > 0);
  const iconButton =
    "relative flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-(--control-radius) p-0 text-sidebar-muted-foreground/80 outline-hidden ring-ring transition-[width,height,padding] hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-inset active:bg-sidebar-row-active disabled:pointer-events-none disabled:opacity-64 [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-(--sidebar-icon-color) hover:[&>svg]:text-sidebar-foreground";
</script>

<div class="flex items-center gap-1">
  <div
    class="flex h-8 min-w-0 flex-1 items-center gap-2 rounded-md px-2 py-1.5 text-sm font-medium text-sidebar-muted-foreground focus-within:bg-sidebar-row-hover focus-within:text-sidebar-foreground focus-within:ring-1 focus-within:ring-ring/70 focus-within:ring-inset hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
  >
    <Search class="size-4 shrink-0 text-(--sidebar-icon-color)" />
    <input
      bind:this={input}
      bind:value={query}
      type="search"
      placeholder="Search"
      aria-label="Search threads"
      role="combobox"
      aria-autocomplete="list"
      aria-expanded={resultsVisible}
      aria-controls={resultsVisible ? "sidebar-thread-search-results" : undefined}
      aria-activedescendant={resultsVisible && activeResult < resultCount ? `sidebar-thread-search-result-${activeResult}` : undefined}
      onkeydown={onKeydown}
      class="h-auto min-w-0 flex-1 bg-transparent p-0 text-sm leading-normal font-medium text-sidebar-foreground outline-none placeholder:text-sidebar-muted-foreground [&::-webkit-search-cancel-button]:hidden"
    />
    {#if searching}
      <button
        type="button"
        aria-label="Clear thread search"
        class="flex size-5 shrink-0 cursor-pointer items-center justify-center rounded-sm text-muted-foreground hover:bg-accent hover:text-foreground"
        onclick={() => {
          query = "";
          input?.focus();
        }}
      >
        <X class="size-3" />
      </button>
    {/if}
  </div>
  <div class="flex shrink-0 items-center">
    {@render projectScope()}
    <button type="button" class={iconButton} aria-label="New thread" title="New thread (Ctrl+N)" onclick={onNewThread} disabled={!hasProjects}>
      <SquarePen />
    </button>
  </div>
</div>
