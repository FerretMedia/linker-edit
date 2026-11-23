<script lang="ts">
  // T3 Code's project folder header (apps/web/src/components/sidebar/SidebarProjectFolderHeader.tsx; MIT, see NOTICE),
  // with Linker-Edit's green "open in Resolve" dot.
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import Monitor from "@lucide/svelte/icons/monitor";
  import SquarePen from "@lucide/svelte/icons/square-pen";
  import { LIBRARY_LABEL } from "#lib/labels.ts";
  import { isUnread, threadStatus, type SidebarProject, type SidebarThread } from "#lib/sidebar.ts";
  import ProjectIcon from "./ProjectIcon.svelte";

  let {
    project,
    machine,
    machineStatus,
    collapsed,
    hiddenThreads,
    onToggle,
    onNewThread,
    onContextMenu,
  }: {
    project: SidebarProject;
    /** The machine it's on, when there's more than one: T3 Code's environment badge. */
    machine: string | null;
    machineStatus: "connecting" | "connected" | "offline";
    collapsed: boolean;
    /** The threads a closed folder hides, for its count and dot. */
    hiddenThreads: SidebarThread[];
    onToggle: () => void;
    onNewThread: () => void;
    onContextMenu: (x: number, y: number) => void;
  } = $props();

  const DOT = {
    attention: { class: "bg-warning", label: ", needs you" },
    unread: { class: "bg-success", label: ", done" },
    working: { class: "bg-info", label: ", working" },
  };

  // The most urgent state among the hidden threads, so a closed folder still says when one needs you.
  const dot = $derived.by(() => {
    if (!collapsed) return null;
    const statuses = new Set(hiddenThreads.map(threadStatus));
    if (statuses.has("approval")) return DOT.attention;
    if (hiddenThreads.some((thread) => threadStatus(thread) === "ready" && isUnread(thread))) return DOT.unread;
    if (statuses.has("working")) return DOT.working;
    return null;
  });

  const tooltip = $derived(
    [
      project.link ? `${project.link.name} · ${LIBRARY_LABEL[project.link.kind]}` : "Not linked to a Resolve project yet",
      project.dir,
      machine && `On ${machine}${project.offline ? ", offline: as it was when last seen" : ""}`,
    ]
      .filter(Boolean)
      .join("\n"),
  );
  const MACHINE_DOT = { connecting: "bg-warning", connected: "bg-success", offline: "bg-muted-foreground" };
</script>

<li
  data-testid="sidebar-project-folder"
  data-project-id={project.id}
  data-machine-id={project.machineId}
  class="group/project-folder relative mt-2 list-none first:mt-0 {project.offline ? 'opacity-60' : ''}"
>
  <button
    type="button"
    aria-expanded={!collapsed}
    title={tooltip}
    onclick={onToggle}
    oncontextmenu={(event) => {
      event.preventDefault();
      onContextMenu(event.clientX, event.clientY);
    }}
    class="flex h-8 w-full cursor-pointer items-center gap-2.5 rounded-(--control-radius) px-(--sidebar-row-content-inset) text-left outline-none hover:bg-sidebar-row-hover focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
  >
    <ProjectIcon icon={project.icon} name={project.name} class="size-4 shrink-0" />
    <span class="min-w-0 flex-1 truncate text-sm font-medium text-sidebar-foreground/90">{project.name}</span>
    {#if project.offline}
      <span class="shrink-0 text-2xs text-sidebar-muted-foreground">offline</span>
    {/if}
    {#if machine}
      <span class="relative flex shrink-0 text-sidebar-muted-foreground" title="On {machine}" data-testid="sidebar-machine-badge">
        <Monitor aria-hidden="true" class="size-3.5" />
        <span class="absolute -right-0.5 -bottom-0.5 size-1.5 rounded-full ring-1 ring-sidebar {MACHINE_DOT[machineStatus]}"></span>
        <span class="sr-only">, on {machine}</span>
      </span>
    {/if}
    {#if project.open}
      <span class="size-1.5 shrink-0 rounded-full bg-success" title="Open in Resolve"><span class="sr-only">, open in Resolve</span></span>
    {/if}
    {#if dot}
      <span class="size-1.5 shrink-0 rounded-full {dot.class}"><span class="sr-only">{dot.label}</span></span>
    {/if}
    {#if collapsed && hiddenThreads.length > 0}
      <span class="shrink-0 text-xs text-sidebar-muted-foreground tabular-nums group-hover/project-folder:invisible">{hiddenThreads.length}</span>
    {/if}
    <ChevronDown aria-hidden="true" class="size-3 shrink-0 text-sidebar-muted-foreground transition-transform {collapsed ? '-rotate-90' : ''}" />
  </button>
  <span
    class="absolute inset-y-0 right-6 flex items-center opacity-0 group-hover/project-folder:opacity-100 focus-within:opacity-100 {project.offline
      ? 'hidden'
      : ''}"
  >
    <button
      type="button"
      aria-label="New thread in {project.name}"
      title="New thread in {project.name}"
      onclick={onNewThread}
      class="inline-flex size-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-accent hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring [&>svg]:size-3.5"
    >
      <SquarePen aria-hidden="true" />
    </button>
  </span>
</li>
