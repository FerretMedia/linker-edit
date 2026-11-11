<script lang="ts">
  import {
    ArchiveArrowUp,
    Archive,
    AudioLines,
    ChevronRight,
    Clapperboard,
    Cloud,
    FolderOpen,
    HardDrive,
    MonitorUp,
    Pencil,
    Plus,
    RefreshCw,
    Server,
    Settings2,
    SquarePen,
    Trash,
  } from "@lucide/svelte";
  import { onMount } from "svelte";
  import type {
    DesktopUpdate,
    LinkerAccount,
    ProjectSummary,
    RemoteAccess,
    ResolveStatus,
    ThreadSummary,
    TranscriptionStatus,
  } from "#shared/protocol.ts";
  import { desktop } from "#lib/desktop.ts";
  import { ago, LIBRARY_LABEL } from "#lib/labels.ts";
  import ComputerPicker from "./ComputerPicker.svelte";
  import LinkerAccountRow from "./LinkerAccount.svelte";
  import RowMenu from "./RowMenu.svelte";

  let {
    resolve,
    version,
    update,
    transcription,
    remote,
    computer,
    linker,
    projects,
    threadId,
    onSelect,
    onNewThread,
    onRenameThread,
    onArchiveThread,
    onDeleteThread,
    onAddProject,
    onAddOpenProject,
    onProjectSettings,
    onOpenInResolve,
    onRefreshResolve,
    onRemoteAccess,
    onLinkerCancel,
    onLinkerSignOut,
  }: {
    resolve: ResolveStatus;
    version: string;
    update: string | null;
    transcription: TranscriptionStatus;
    remote: RemoteAccess;
    computer: string;
    linker: LinkerAccount;
    projects: ProjectSummary[];
    threadId: string | null;
    onSelect: (threadId: string) => void;
    onNewThread: (projectId: string) => void;
    onRenameThread: (threadId: string, title: string) => void;
    onArchiveThread: (threadId: string, archived: boolean) => void;
    onDeleteThread: (threadId: string) => void;
    onAddProject: () => void;
    onAddOpenProject: () => void;
    onProjectSettings: (projectId: string) => void;
    onOpenInResolve: (projectId: string) => void;
    onRefreshResolve: () => void;
    onRemoteAccess: () => void;
    onLinkerCancel: () => void;
    onLinkerSignOut: () => void;
  } = $props();

  // The installed app updates itself; `update` is the npm release, for `npx linker-edit`.
  let appUpdate = $state<DesktopUpdate | null>(null);
  onMount(() => {
    void desktop?.update?.().then((next) => (appUpdate = next));
    return desktop?.onUpdate?.((next) => (appUpdate = next));
  });

  const COLLAPSED_KEY = "linker-edit.collapsed-projects";
  let collapsed = $state(new Set<string>(JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]") as string[]));
  let showArchived = $state(new Set<string>());
  let renaming = $state<string | null>(null);
  let renameText = $state("");

  // Recomputed when the list changes, so "5m" ages without a ticking clock.
  const now = $derived.by(() => {
    void projects;
    return new Date();
  });

  const status = $derived.by(() => {
    switch (resolve.state) {
      case "checking":
        return { tone: "bg-warning", title: "Checking Resolve…", detail: "" };
      case "connected":
        return {
          tone: "bg-success",
          title: `${resolve.product} ${resolve.version.split(".").slice(0, 3).join(".")}`,
          detail: resolve.project
            ? `${resolve.project}${resolve.timeline ? ` · ${resolve.timeline}` : ""}`
            : "No project open",
        };
      case "not_running":
        return { tone: "bg-muted-foreground", title: "Resolve is not running", detail: "Start Resolve, then refresh." };
      case "blocked":
        return { tone: "bg-warning", title: "Resolve needs attention", detail: resolve.hint };
      case "unreachable":
        return { tone: "bg-destructive", title: "Resolve not reachable", detail: resolve.hint };
      case "setup_needed":
        return { tone: "bg-destructive", title: "Setup needed", detail: resolve.hint };
    }
  });

  // The project open in Resolve when no Linker-Edit project has it yet.
  const transcribing = $derived.by(() => {
    switch (transcription.state) {
      case "idle":
        return null;
      case "preparing":
        return {
          title: transcription.percent === null ? transcription.step : `${transcription.step} · ${transcription.percent}%`,
          detail: "Only the first time",
        };
      case "transcribing":
        return {
          title: "Transcribing",
          detail: transcription.total > 1 ? `${transcription.file} (${transcription.done + 1} of ${transcription.total})` : transcription.file,
        };
    }
  });

  const unlinkedOpen = $derived(
    resolve.state === "connected" && resolve.project && !projects.some((project) => project.open) ? resolve.project : null,
  );

  const KIND_ICON = { local: HardDrive, network: Server, cloud: Cloud };

  function toggle(set: Set<string>, id: string): Set<string> {
    const next = new Set(set);
    if (!next.delete(id)) next.add(id);
    return next;
  }

  function toggleCollapsed(projectId: string) {
    collapsed = toggle(collapsed, projectId);
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...collapsed]));
  }

  function startRename(thread: ThreadSummary) {
    renaming = thread.id;
    renameText = thread.title;
  }

  function finishRename(save: boolean) {
    if (renaming && save && renameText.trim()) onRenameThread(renaming, renameText.trim());
    renaming = null;
  }

  function deleteThread(thread: ThreadSummary) {
    if (confirm(`Delete "${thread.title}"? The chat is gone for good; Resolve and the edit history keep what it changed.`)) {
      onDeleteThread(thread.id);
    }
  }

  function selectFocus(node: HTMLInputElement) {
    node.focus();
    node.select();
  }
</script>

{#snippet threadRow(thread: ThreadSummary)}
  {@const active = thread.id === threadId}
  <div
    class="group/thread flex h-8 items-center gap-1 rounded-lg pr-1 pl-7 text-sm transition-colors {active
      ? 'bg-sidebar-row-active shadow-[0_0_0_1px_var(--border)]'
      : 'hover:bg-accent'}"
  >
    {#if renaming === thread.id}
      <input
        class="h-6 min-w-0 flex-1 rounded-md border bg-card px-1.5 text-sm outline-none"
        bind:value={renameText}
        use:selectFocus
        onkeydown={(event) => {
          if (event.key === "Enter") finishRename(true);
          if (event.key === "Escape") finishRename(false);
        }}
        onblur={() => finishRename(true)}
      />
    {:else}
      <button
        type="button"
        class="flex h-full min-w-0 flex-1 items-center gap-2 text-left {thread.archived ? 'text-muted-foreground' : ''}"
        aria-current={active ? "page" : undefined}
        onclick={() => onSelect(thread.id)}
        ondblclick={() => startRename(thread)}
      >
        <span class="min-w-0 flex-1 truncate">{thread.title}</span>
        {#if thread.waiting}
          <span class="size-1.5 shrink-0 rounded-full bg-warning" title="Waiting for your approval"></span>
        {:else if thread.busy}
          <span class="size-1.5 shrink-0 rounded-full bg-primary" title="Working"></span>
        {:else}
          <span class="shrink-0 text-2xs text-muted-foreground group-hover/thread:hidden">{ago(new Date(thread.updatedAt), now)}</span>
        {/if}
      </button>
      <div class="hidden group-hover/thread:block has-[[aria-expanded=true]]:block">
        <RowMenu
          label="Thread actions"
          items={[
            { label: "Rename", Icon: Pencil, onSelect: () => startRename(thread) },
            thread.archived
              ? { label: "Unarchive", Icon: ArchiveArrowUp, onSelect: () => onArchiveThread(thread.id, false) }
              : { label: "Archive", Icon: Archive, onSelect: () => onArchiveThread(thread.id, true) },
            { label: "Delete", Icon: Trash, danger: true, onSelect: () => deleteThread(thread) },
          ]}
        />
      </div>
    {/if}
  </div>
{/snippet}

<aside class="flex w-64 shrink-0 flex-col border-r bg-sidebar">
  <div class="title-bar title-bar-start flex h-13 shrink-0 items-center gap-2 px-4">
    <span class="flex size-6 items-center justify-center rounded-md bg-primary text-primary-foreground">
      <Clapperboard class="size-3.5" strokeWidth={2.25} />
    </span>
    <span class="text-sm font-semibold tracking-tight">Linker-Edit</span>
    <span class="ml-auto text-2xs text-muted-foreground">{version === "0.0.0" ? "dev" : version}</span>
  </div>

  <ComputerPicker {computer} {remote} {onRemoteAccess} />

  <nav class="flex flex-1 flex-col gap-0.5 overflow-y-auto px-2 pb-2">
    <div class="flex h-7 items-center gap-2 pr-1 pl-2.5 text-xs font-medium text-muted-foreground">
      <span class="flex-1">Projects</span>
      <button
        type="button"
        class="flex size-6 items-center justify-center rounded-md transition-colors hover:bg-accent hover:text-foreground"
        aria-label="Add project"
        title="Add project"
        onclick={onAddProject}
      >
        <Plus class="size-3.5" />
      </button>
    </div>

    {#if unlinkedOpen}
      <button
        type="button"
        class="mb-1 flex items-center gap-2 rounded-lg border border-dashed px-2.5 py-1.5 text-left text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        onclick={onAddOpenProject}
      >
        <Plus class="size-3.5 shrink-0" />
        <span class="min-w-0 flex-1">Add <span class="font-medium text-foreground">{unlinkedOpen}</span>, open in Resolve</span>
      </button>
    {/if}

    {#each projects as project (project.id)}
      {@const KindIcon = project.link ? KIND_ICON[project.link.kind] : FolderOpen}
      {@const active = project.threads.filter((thread) => !thread.archived)}
      {@const archived = project.threads.filter((thread) => thread.archived)}
      {@const isCollapsed = collapsed.has(project.id)}
      <div class="group/project flex h-8 items-center gap-1 rounded-lg pr-1 pl-1 text-sm hover:bg-accent">
        <button
          type="button"
          class="flex h-full min-w-0 flex-1 items-center gap-1.5 text-left"
          aria-expanded={!isCollapsed}
          onclick={() => toggleCollapsed(project.id)}
          title={project.link ? `${project.link.name} · ${LIBRARY_LABEL[project.link.kind]}` : "Not linked to a Resolve project yet"}
        >
          <ChevronRight class="size-3.5 shrink-0 text-muted-foreground transition-transform {isCollapsed ? '' : 'rotate-90'}" />
          <KindIcon class="size-3.5 shrink-0 text-muted-foreground" />
          <span class="min-w-0 flex-1 truncate font-medium">{project.name}</span>
          {#if project.open}
            <span class="size-1.5 shrink-0 rounded-full bg-success" title="Open in Resolve"></span>
          {/if}
        </button>
        <div class="hidden items-center group-hover/project:flex has-[[aria-expanded=true]]:flex">
          <button
            type="button"
            class="flex size-6 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            aria-label="New thread in {project.name}"
            title="New thread"
            onclick={() => onNewThread(project.id)}
          >
            <SquarePen class="size-3.5" />
          </button>
          <RowMenu
            label="Project actions"
            items={[
              { label: "New thread", Icon: SquarePen, onSelect: () => onNewThread(project.id) },
              ...(project.link && !project.open
                ? [{ label: "Open in Resolve", Icon: MonitorUp, onSelect: () => onOpenInResolve(project.id) }]
                : []),
              { label: "Project settings", Icon: Settings2, onSelect: () => onProjectSettings(project.id) },
            ]}
          />
        </div>
      </div>

      {#if !isCollapsed}
        {#each active as thread (thread.id)}
          {@render threadRow(thread)}
        {/each}
        {#if archived.length}
          <button
            type="button"
            class="flex h-7 items-center gap-1.5 rounded-lg pl-7 text-left text-xs text-muted-foreground transition-colors hover:text-foreground"
            aria-expanded={showArchived.has(project.id)}
            onclick={() => (showArchived = toggle(showArchived, project.id))}
          >
            <Archive class="size-3" />
            Archived ({archived.length})
          </button>
          {#if showArchived.has(project.id)}
            {#each archived as thread (thread.id)}
              {@render threadRow(thread)}
            {/each}
          {/if}
        {/if}
      {/if}
    {/each}
  </nav>

  {#if transcribing}
    <div class="mx-2 flex items-start gap-2.5 px-2.5 py-2" title={transcribing.detail}>
      <AudioLines class="mt-0.5 size-3.5 shrink-0 text-primary" />
      <span class="min-w-0 flex-1">
        <span class="block truncate text-xs font-medium">{transcribing.title}</span>
        <span class="mt-0.5 block truncate text-2xs text-muted-foreground">{transcribing.detail}</span>
      </span>
    </div>
  {/if}

  {#if appUpdate}
    <div class="mx-2 rounded-lg px-2.5 py-2 shadow-[0_0_0_1px_var(--border)]">
      {#if appUpdate.state === "ready"}
        <span class="block text-xs font-medium">Linker-Edit {appUpdate.version} is ready</span>
        <button
          type="button"
          class="mt-0.5 text-2xs text-primary hover:underline"
          onclick={() => void desktop?.installUpdate?.()}
        >
          Restart to update
        </button>
      {:else}
        <span class="block text-xs font-medium">Linker-Edit {appUpdate.version} is out</span>
        <a class="mt-0.5 block text-2xs text-primary hover:underline" href={appUpdate.url} target="_blank" rel="noreferrer">
          Download it
        </a>
      {/if}
    </div>
  {:else if update}
    <div class="mx-2 rounded-lg px-2.5 py-2 shadow-[0_0_0_1px_var(--border)]">
      <span class="block text-xs font-medium">Linker-Edit {update} is out</span>
      <span class="mt-0.5 block text-2xs text-muted-foreground">
        Stop this one, then run <code class="font-mono">npx linker-edit@latest</code>.
      </span>
    </div>
  {/if}

  <div class="mt-2">
    <LinkerAccountRow {linker} {computer} onCancel={onLinkerCancel} onSignOut={onLinkerSignOut} />
  </div>

  <button
    type="button"
    class="group m-2 mt-0.5 flex items-start gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-accent"
    onclick={onRefreshResolve}
    title="Refresh Resolve status"
  >
    <span class="mt-1.5 size-2 shrink-0 rounded-full {status.tone}"></span>
    <span class="min-w-0 flex-1">
      <span class="block truncate text-xs font-medium">{status.title}</span>
      {#if status.detail}
        <span class="mt-0.5 block text-2xs text-muted-foreground">{status.detail}</span>
      {/if}
    </span>
    <RefreshCw class="mt-0.5 size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
  </button>
</aside>
