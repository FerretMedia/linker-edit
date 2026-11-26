<script lang="ts">
  // T3 Code's project scope (the Projects combobox in apps/web/src/components/Sidebar.tsx, ProjectScopeRowIcon,
  // ProjectRowActions, ProjectPinButton and ProjectEnvironmentBadge; MIT, see NOTICE): picks which projects' threads
  // the sidebar lists. A click lists one project, Shift+click adds or takes one away, All projects lists them all.
  import Check from "@lucide/svelte/icons/check";
  import Folder from "@lucide/svelte/icons/folder";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";
  import Monitor from "@lucide/svelte/icons/monitor";
  import Pin from "@lucide/svelte/icons/pin";
  import PinOff from "@lucide/svelte/icons/pin-off";
  import Search from "@lucide/svelte/icons/search";
  import Settings from "@lucide/svelte/icons/settings";
  import { tick } from "svelte";
  import { syncText, type SidebarProject } from "#lib/sidebar.ts";
  import DriveSyncState from "./DriveSyncState.svelte";
  import ProjectIcon from "./ProjectIcon.svelte";

  let {
    projects,
    scope,
    machineOf,
    machineStatus,
    onScope,
    onAddProject,
    onPin,
    onProjectSettings,
    onContextMenu,
  }: {
    /** In the sidebar's order: pinned first. */
    projects: SidebarProject[];
    /** The listed projects' keys; empty lists every project. */
    scope: string[];
    /** A project's machine, when there's more than one. */
    machineOf: (project: SidebarProject) => string | null;
    machineStatus: (project: SidebarProject) => "connecting" | "connected" | "offline";
    onScope: (keys: string[]) => void;
    onAddProject: () => void;
    onPin: (project: SidebarProject) => void;
    onProjectSettings: (project: SidebarProject) => void;
    onContextMenu: (project: SidebarProject, x: number, y: number) => void;
  } = $props();

  type Item = { value: "add" | "all"; label: string } | { value: string; label: string; project: SidebarProject };

  let open = $state(false);
  let query = $state("");
  let highlighted = $state(0);
  let trigger: HTMLButtonElement | undefined = $state();
  let popup: HTMLDivElement | undefined = $state();
  let position = $state({ left: 0, top: 0 });

  const scoped = $derived(projects.filter((project) => scope.includes(project.key)));
  const label = $derived(scoped.length > 0 ? `Projects: ${scoped.map((project) => project.name).join(", ")}` : "Projects");
  const fold = (text: string) => text.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();
  const items = $derived.by((): Item[] => {
    const all: Item[] = [
      { value: "add", label: "Add project" },
      { value: "all", label: "All projects" },
      ...projects.filter((project) => !project.archived).map((project) => ({ value: project.key, label: project.name, project })),
    ];
    const words = fold(query).split(/\s+/).filter(Boolean);
    return words.length === 0 ? all : all.filter((item) => words.every((word) => fold(item.label).includes(word)));
  });
  const MACHINE_DOT = { connecting: "bg-warning", connected: "bg-success", offline: "bg-muted-foreground" };

  async function show() {
    query = "";
    open = true;
    await tick();
    if (!trigger || !popup) return;
    const rect = trigger.getBoundingClientRect();
    const width = popup.offsetWidth;
    position = { left: Math.max(8, Math.min(rect.left, innerWidth - width - 8)), top: rect.bottom + 4 };
    highlighted = Math.max(0, items.findIndex((item) => (scope.length === 0 ? item.value === "all" : item.value === scope[0])));
    popup.querySelector<HTMLInputElement>("input")?.focus();
  }

  function close(refocus = false) {
    open = false;
    if (refocus) trigger?.focus();
  }

  function choose(item: Item | undefined, toggle: boolean) {
    if (!item) return;
    if (item.value === "add") {
      close();
      onAddProject();
    } else if (item.value === "all") {
      onScope([]);
      close();
    } else if (toggle) {
      // Stays open, to pick more.
      onScope(scope.includes(item.value) ? scope.filter((key) => key !== item.value) : [...scope, item.value]);
    } else {
      // The only project listed, clicked again, lists them all.
      onScope(scope.length === 1 && scope[0] === item.value ? [] : [item.value]);
      close();
    }
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      close(true);
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault();
      if (items.length > 0) highlighted = (highlighted + (event.key === "ArrowDown" ? 1 : -1) + items.length) % items.length;
    } else if (event.key === "Enter") {
      event.preventDefault();
      choose(items[highlighted], event.shiftKey);
    }
  }

  $effect(() => {
    void query;
    highlighted = Math.min(highlighted, Math.max(0, items.length - 1));
  });

  function onWindowPointerDown(event: PointerEvent) {
    if (!open) return;
    const target = event.target as Node;
    if (popup?.contains(target) || trigger?.contains(target)) return;
    close();
  }

  const iconButton =
    "relative flex size-7 shrink-0 cursor-pointer items-center justify-center rounded-(--control-radius) p-0 text-sidebar-muted-foreground/80 outline-hidden ring-ring transition-[width,height,padding] hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-inset active:bg-sidebar-row-active [&>svg]:size-4 [&>svg]:shrink-0 [&>svg]:text-(--sidebar-icon-color) hover:[&>svg]:text-sidebar-foreground";
  const rowAction =
    "inline-flex size-6 cursor-pointer items-center justify-center rounded-md text-muted-foreground outline-none hover:bg-foreground/[0.06] hover:text-foreground";
</script>

<svelte:window onpointerdown={onWindowPointerDown} onresize={() => close()} />

<button
  bind:this={trigger}
  type="button"
  class={iconButton}
  aria-label={label}
  title={label}
  aria-haspopup="listbox"
  aria-expanded={open}
  data-testid="sidebar-project-scope"
  onclick={() => (open ? close() : void show())}
>
  {#if scoped.length > 1}
    <span class="flex shrink-0">
      {#each scoped.slice(0, 2) as project, index (project.key)}
        <span class={index === 0 ? "flex" : "-ml-1 flex rounded-[25%] bg-sidebar ring-2 ring-sidebar"}>
          <ProjectIcon icon={project.icon} name={project.name} class="size-3.5" />
        </span>
      {/each}
    </span>
  {:else if scoped[0]}
    <span class="flex shrink-0"><ProjectIcon icon={scoped[0].icon} name={scoped[0].name} class="size-4" /></span>
  {:else}
    <Folder />
  {/if}
</button>

{#if open}
  <div
    bind:this={popup}
    class="fixed z-[120] flex max-h-[min(28rem,calc(100vh-4rem))] w-[min(18rem,calc(100vw-1rem))] flex-col overflow-hidden rounded-lg border border-foreground/10 bg-popover text-foreground shadow-[0_16px_40px_-18px_rgb(0_0_0/55%)] dark:shadow-[0_18px_44px_-18px_rgb(0_0_0/80%)]"
    style="left: {position.left}px; top: {position.top}px"
  >
    <label class="block min-w-0 shrink-0 cursor-text px-3 pt-2.5">
      <span class="relative block -translate-y-px border-b border-border/70 pb-1.5 transition-colors focus-within:border-ring">
        <Search aria-hidden="true" class="pointer-events-none absolute top-1.5 left-0 size-4 shrink-0 text-muted-foreground/55" />
        <input
          bind:value={query}
          type="text"
          role="combobox"
          aria-label="Search projects"
          aria-expanded="true"
          aria-controls="sidebar-project-scope-list"
          aria-activedescendant={items[highlighted] ? `sidebar-project-scope-${highlighted}` : undefined}
          placeholder="Search projects..."
          onkeydown={onKeydown}
          class="h-6.5 w-full bg-transparent ps-5 text-sm leading-6.5 outline-none placeholder:text-muted-foreground/70"
        />
      </span>
    </label>
    {#if items.length === 0}
      <p class="px-3 py-2 text-xs font-medium text-muted-foreground">No matching projects.</p>
    {/if}
    <ul id="sidebar-project-scope-list" role="listbox" aria-label="Projects" aria-multiselectable="true" class="min-h-0 overflow-y-auto p-1">
      {#each items as item, index (item.value)}
        {@const project = "project" in item ? item.project : null}
        {@const selected = project ? scope.includes(item.value) : item.value === "all" && scope.length === 0}
        {@const machine = project ? machineOf(project) : null}
        <!-- The keys go through the search field, as in a combobox; the row holds buttons, so it isn't one. -->
        <!-- svelte-ignore a11y_click_events_have_key_events -->
        <li
          id="sidebar-project-scope-{index}"
          role="option"
          aria-label="{item.label}{project?.open ? ', open in Resolve' : ''}{machine ? `, on ${machine}` : ''}{project?.offline ? ', offline' : ''}{project?.sync
            ? `, ${syncText(project.sync)}`
            : ''}"
          aria-selected={item.value === "add" ? undefined : selected}
          data-highlighted={index === highlighted || undefined}
          data-testid={project ? "sidebar-scope-project" : undefined}
          class="group/scope-row flex min-h-7 cursor-pointer items-center gap-2 rounded-sm px-2 py-1 text-sm outline-none select-none hover:bg-foreground/[0.09] data-highlighted:bg-foreground/[0.09] {project?.offline
            ? 'opacity-60'
            : ''}"
          onpointermove={() => (highlighted = index)}
          onclick={(event) => choose(item, event.shiftKey)}
          oncontextmenu={(event) => {
            if (!project) return;
            event.preventDefault();
            close();
            onContextMenu(project, event.clientX, event.clientY);
          }}
        >
          {#if item.value === "add"}
            <FolderPlus class="size-4 shrink-0 text-muted-foreground" />
          {:else}
            <span class="relative flex shrink-0">
              {#if project}
                <ProjectIcon icon={project.icon} name={project.name} class="size-4 shrink-0" />
              {:else}
                <Folder class="size-4 shrink-0 text-muted-foreground" />
              {/if}
              {#if selected}
                <span class="pointer-events-none absolute -right-1 -bottom-1 grid size-3 place-items-center rounded-full bg-primary ring-2 ring-popover">
                  <Check class="size-2 text-primary-foreground" strokeWidth={4} aria-hidden="true" />
                </span>
              {/if}
            </span>
          {/if}
          <span class="min-w-0 truncate">{item.label}</span>
          {#if project?.open}
            <span class="size-1.5 shrink-0 rounded-full bg-success" title="Open in Resolve"></span>
          {/if}
          {#if project && machine}
            <span class="relative flex shrink-0 text-muted-foreground" title="On {machine}" data-testid="sidebar-machine-badge">
              <Monitor aria-hidden="true" class="size-3.5" />
              <span class="absolute -right-0.5 -bottom-0.5 size-1.5 rounded-full ring-1 ring-popover {MACHINE_DOT[machineStatus(project)]}"></span>
            </span>
          {/if}
          {#if project?.sync}
            <DriveSyncState sync={project.sync} class="text-muted-foreground" />
          {/if}
          <span class="ml-auto flex shrink-0 items-center">
            {#if project && !project.offline}
              <span class="invisible flex group-data-highlighted/scope-row:visible">
                <button
                  type="button"
                  tabindex="-1"
                  class={rowAction}
                  title="Project settings for {project.name}"
                  aria-label="Project settings for {project.name}"
                  onclick={(event) => {
                    event.stopPropagation();
                    close();
                    onProjectSettings(project);
                  }}
                >
                  <Settings class="size-3.5" />
                </button>
              </span>
              <span class="group/pin flex {project.pinned ? '' : 'opacity-0 group-data-highlighted/scope-row:opacity-100'}">
                <button
                  type="button"
                  tabindex="-1"
                  class={rowAction}
                  title={project.pinned ? `Unpin ${project.name}` : `Pin ${project.name}`}
                  aria-label={project.pinned ? `Unpin ${project.name}` : `Pin ${project.name}`}
                  onclick={(event) => {
                    event.stopPropagation();
                    onPin(project);
                  }}
                >
                  {#if project.pinned}
                    <Pin class="size-3.5 group-hover/pin:hidden" />
                    <PinOff class="hidden size-3.5 group-hover/pin:block" />
                  {:else}
                    <Pin class="size-3.5" />
                  {/if}
                </button>
              </span>
            {:else}
              <!-- Keeps every row as tall as the project rows. -->
              <span class="size-6 shrink-0"></span>
            {/if}
          </span>
        </li>
      {/each}
    </ul>
  </div>
{/if}
