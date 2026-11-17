<script lang="ts">
  // T3 Code's shelf header (CollapsibleSectionHeader and SidebarSectionHeader; MIT, see NOTICE).
  import { ChevronDown } from "@lucide/svelte";

  let { label, count, expanded, onToggle, class: className = "" }: { label: string; count: number; expanded: boolean; onToggle: () => void; class?: string } =
    $props();
</script>

<li class="relative mx-0.5 h-8 list-none {className}" data-testid="sidebar-{label.toLowerCase()}-header">
  <button
    type="button"
    aria-expanded={expanded}
    data-testid="sidebar-{label.toLowerCase()}-shelf-toggle"
    onclick={onToggle}
    class="flex h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left text-xs font-medium text-sidebar-muted-foreground focus-visible:ring-2 focus-visible:ring-ring/70 focus-visible:ring-inset focus-visible:outline-none"
  >
    <span class="min-w-0 truncate">{label}</span>
    <span aria-hidden="true" class="invisible h-px min-w-2 flex-1 bg-sidebar-border/50"></span>
    <span class="font-normal tabular-nums">{count}</span>
    <ChevronDown aria-hidden="true" class="size-3 shrink-0 transition-transform {expanded ? '' : '-rotate-90'}" />
  </button>
</li>
