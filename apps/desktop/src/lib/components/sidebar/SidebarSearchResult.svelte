<script lang="ts">
  // T3 Code's SidebarSearchResultRow (MIT, see NOTICE). Settled and archived threads are found too, dimmed.
  import { activityTimestamp, relativeTime, type SidebarThread } from "#lib/sidebar.ts";
  import ProjectIcon from "./ProjectIcon.svelte";

  let {
    thread,
    index,
    machine,
    highlighted,
    active,
    nowMs,
    onHighlight,
    onSelect,
    onContextMenu,
  }: {
    thread: SidebarThread;
    index: number;
    /** The machine it's on, when there's more than one. */
    machine: string | null;
    highlighted: boolean;
    active: boolean;
    nowMs: number;
    onHighlight: () => void;
    onSelect: () => void;
    onContextMenu: (x: number, y: number) => void;
  } = $props();

  const parked = $derived(!!thread.settledAt || thread.archived || thread.project.offline);
  const scrollIntoView = (node: HTMLElement, on: boolean) => {
    if (on) node.scrollIntoView({ block: "nearest" });
    return { update: (next: boolean) => next && node.scrollIntoView({ block: "nearest" }) };
  };
</script>

<li role="presentation" class="list-none" use:scrollIntoView={highlighted}>
  <button
    id="sidebar-thread-search-result-{index}"
    type="button"
    role="option"
    tabindex="-1"
    aria-selected={highlighted}
    aria-current={active ? "page" : undefined}
    aria-label="{thread.title}, {thread.project.name}{machine ? `, on ${machine}` : ''}"
    data-testid="sidebar-search-result"
    onmousemove={onHighlight}
    onpointerdown={(event) => event.preventDefault()}
    onclick={onSelect}
    oncontextmenu={(event) => {
      event.preventDefault();
      onContextMenu(event.clientX, event.clientY);
    }}
    class="group/sidebar-row flex min-h-9 w-full cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-1 text-left text-sm outline-none {highlighted || active
      ? 'bg-sidebar-row-active text-sidebar-foreground'
      : 'text-sidebar-muted-foreground/75 hover:bg-sidebar-row-hover hover:text-sidebar-foreground'}"
  >
    <ProjectIcon
      icon={thread.project.icon}
      name={thread.project.name}
      class="size-4 shrink-0 transition-opacity {parked && !active && !highlighted
        ? 'opacity-40 grayscale group-hover/sidebar-row:opacity-100 group-hover/sidebar-row:grayscale-0'
        : ''}"
    />
    <span class="flex min-w-0 flex-1 items-center gap-2.5">
      <span
        class="min-w-0 flex-1 truncate {parked ? 'font-normal text-secondary-label/70 group-hover/sidebar-row:text-foreground' : 'font-medium text-foreground'} {highlighted ||
        active
          ? 'text-foreground'
          : ''}"
      >
        {thread.title}
      </span>
      {#if thread.archived}
        <span class="shrink-0 text-2xs text-muted-foreground/70">Archived</span>
      {/if}
      {#if machine}
        <span class="max-w-20 shrink-0 truncate text-2xs text-muted-foreground/70">{thread.project.offline ? `${machine}, offline` : machine}</span>
      {/if}
      <span class="w-8 shrink-0 text-right text-xs text-muted-foreground/55 tabular-nums">{relativeTime(activityTimestamp(thread), nowMs)}</span>
    </span>
  </button>
</li>
