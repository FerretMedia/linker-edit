<script lang="ts">
  // A copy of T3 Code's sidebar (apps/web/src/components/Sidebar.tsx and sidebar/*; MIT, see NOTICE), in its
  // project-folder layout: pinned threads, a folder per project, then the Snoozed and Settled shelves. Linker-Edit's
  // own parts (computer picker, Resolve, transcription, updates, Linker account) stay around it.
  import {
    Archive,
    ArchiveRestore,
    ChevronDown,
    ChevronUp,
    CircleCheck,
    Clock,
    Copy,
    Folder,
    Hash,
    MailOpen,
    MonitorUp,
    Pencil,
    Pin,
    PinOff,
    Plus,
    Settings,
    SquarePen,
    Trash,
  } from "@lucide/svelte";
  import type {
    AppUpdate,
    ClientMessage,
    LinkerAccount,
    ProjectSummary,
    RemoteAccess,
    ResolveStatus,
    TranscriptionStatus,
  } from "#shared/protocol.ts";
  import { releaseName } from "#shared/release-name.ts";
  import {
    FOLDER_PREVIEW_COUNT,
    SETTLED_INITIAL_COUNT,
    SETTLED_PAGE_COUNT,
    isUnread,
    isWoke,
    nextThreadAfter,
    nextWake,
    partitionSidebar,
    searchThreads,
    snoozePresets,
    type SidebarThread,
  } from "#lib/sidebar.ts";
  import ComputerPicker from "./ComputerPicker.svelte";
  import LinkerLogo from "./LinkerLogo.svelte";
  import ContextMenu, { type ContextMenuItem } from "./sidebar/ContextMenu.svelte";
  import SidebarFooter from "./sidebar/SidebarFooter.svelte";
  import SidebarProject from "./sidebar/SidebarProject.svelte";
  import SidebarSearch from "./sidebar/SidebarSearch.svelte";
  import SidebarSearchResult from "./sidebar/SidebarSearchResult.svelte";
  import SidebarShelfHeader from "./sidebar/SidebarShelfHeader.svelte";
  import SidebarThreadRow from "./sidebar/SidebarThreadRow.svelte";

  let {
    resolve,
    resolveHere,
    version,
    update,
    appUpdate,
    transcription,
    remote,
    computer,
    linker,
    projects,
    threadId,
    send,
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
    onResolveHere,
    onRemoteAccess,
    onSettings,
    onLinkerCancel,
    onLinkerSignOut,
    onTranscriptionSetup,
    onTranscriptionCancel,
    onCheckAppUpdate,
    onInstallAppUpdate,
    onCancelAppUpdate,
  }: {
    resolve: ResolveStatus;
    resolveHere: boolean;
    version: string;
    update: string | null;
    appUpdate: AppUpdate | null;
    transcription: TranscriptionStatus;
    remote: RemoteAccess;
    computer: string;
    linker: LinkerAccount;
    projects: ProjectSummary[];
    threadId: string | null;
    /** For pinning, settling, snoozing, reading, and project icons, pins and archiving. */
    send: (message: ClientMessage) => void;
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
    onResolveHere: (resolveHere: boolean) => void;
    onRemoteAccess: () => void;
    onSettings: () => void;
    onLinkerCancel: () => void;
    onLinkerSignOut: () => void;
    onTranscriptionSetup: () => void;
    onTranscriptionCancel: () => void;
    onCheckAppUpdate: () => void;
    onInstallAppUpdate: () => void;
    onCancelAppUpdate: () => void;
  } = $props();

  // The desktop app updates itself (appUpdate); `update` is the npm release, for `npx linker-edit`.
  // After "Check for updates", the card also says when there's nothing new, for a few seconds.
  let asked = $state(false);
  $effect(() => {
    if (!asked || appUpdate?.status === "checking") return;
    const timer = setTimeout(() => (asked = false), 6000);
    return () => clearTimeout(timer);
  });

  function checkForUpdates() {
    asked = true;
    onCheckAppUpdate();
  }

  // A minute clock, like T3 Code's, and an exact tick when a snoozed thread wakes.
  let nowMs = $state(Date.now());
  $effect(() => {
    const timer = setInterval(() => (nowMs = Date.now()), 60_000);
    return () => clearInterval(timer);
  });
  $effect(() => {
    void projects;
    nowMs = Date.now();
  });
  $effect(() => {
    const wake = nextWake(projects, nowMs);
    if (wake === null) return;
    const timer = setTimeout(() => (nowMs = Date.now()), Math.min(wake - Date.now() + 50, 2 ** 31 - 1));
    return () => clearTimeout(timer);
  });

  const lists = $derived(partitionSidebar(projects, nowMs));
  const current = $derived(projects.flatMap((project) => project.threads.map((thread) => ({ ...thread, project }))).find((thread) => thread.id === threadId) ?? null);

  function stored<T>(key: string, fallback: T): T {
    try {
      const saved = localStorage.getItem(key);
      return saved === null ? fallback : (JSON.parse(saved) as T);
    } catch {
      return fallback;
    }
  }
  function store(key: string, value: unknown) {
    try {
      localStorage.setItem(key, JSON.stringify(value));
    } catch {
      // A private window only forgets what's open after a reload.
    }
  }

  // Kept under the old sidebar's key, so folders closed before stay closed, as T3 Code does with its old sidebar's.
  const COLLAPSED_KEY = "linker-edit.collapsed-projects";
  const SETTLED_KEY = "linker-edit.sidebar.settled-expanded";
  const SNOOZED_KEY = "linker-edit.sidebar.snoozed-expanded";
  let collapsed = $state(new Set<string>(stored<string[]>(COLLAPSED_KEY, [])));
  let settledExpanded = $state(stored(SETTLED_KEY, false));
  let snoozedExpanded = $state(stored(SNOOZED_KEY, false));
  let expandedFolders = $state(new Set<string>());
  let settledShown = $state(SETTLED_INITIAL_COUNT);

  function toggleFolder(projectId: string) {
    const next = new Set(collapsed);
    if (!next.delete(projectId)) next.add(projectId);
    collapsed = next;
    store(COLLAPSED_KEY, [...next]);
  }

  function toggleShowAll(projectId: string) {
    const next = new Set(expandedFolders);
    if (!next.delete(projectId)) next.add(projectId);
    expandedFolders = next;
  }

  /** A folder's first six, and the open thread when it's further down. */
  function folderRows(folder: { project: ProjectSummary; threads: SidebarThread[] }) {
    if (expandedFolders.has(folder.project.id) || folder.threads.length <= FOLDER_PREVIEW_COUNT) return { rows: folder.threads, hidden: 0 };
    const rows = folder.threads.filter((thread, index) => index < FOLDER_PREVIEW_COUNT || thread.id === threadId);
    return { rows, hidden: folder.threads.length - rows.length };
  }

  // --- Search ---
  let query = $state("");
  let searchInput: HTMLInputElement | undefined = $state();
  let activeResult = $state(0);
  const searching = $derived(query.trim().length > 0);
  const results = $derived(searching ? searchThreads(projects, query) : []);
  $effect(() => {
    void query;
    activeResult = 0;
  });

  function openResult(thread: SidebarThread) {
    query = "";
    onSelect(thread.id);
  }

  function onSearchKeydown(event: KeyboardEvent) {
    if (event.isComposing) return;
    if (event.key === "Escape") {
      event.preventDefault();
      query = "";
      searchInput?.blur();
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      if (results.length === 0) return;
      event.preventDefault();
      activeResult = (activeResult + (event.key === "ArrowDown" ? 1 : -1) + results.length) % results.length;
    } else if (event.key === "Enter") {
      const thread = results[activeResult];
      if (!thread) return;
      event.preventDefault();
      openResult(thread);
    }
  }

  // --- Thread actions ---
  let renaming = $state<string | null>(null);

  /** Settling or snoozing the open thread opens the next listed one, or a new thread in its project. As in T3 Code. */
  function moveOnFrom(thread: SidebarThread) {
    if (thread.id !== threadId) return;
    const next = nextThreadAfter(lists.order, thread.id);
    if (next) onSelect(next.id);
    else onNewThread(thread.project.id);
  }

  function settle(thread: SidebarThread, settled: boolean) {
    if (settled) moveOnFrom(thread);
    send({ type: "settle_thread", threadId: thread.id, settled });
  }

  function snooze(thread: SidebarThread, until: string | null) {
    if (until) moveOnFrom(thread);
    send({ type: "snooze_thread", threadId: thread.id, until });
  }

  const pin = (thread: SidebarThread, pinned: boolean) => send({ type: "pin_thread", threadId: thread.id, pinned });

  function deleteThread(thread: SidebarThread) {
    if (confirm(`Delete "${thread.title}"? The chat is gone for good; Resolve and the edit history keep what it changed.`)) {
      onDeleteThread(thread.id);
    }
  }

  function newThread() {
    const projectId = current?.project.id ?? lists.folders[0]?.project.id ?? projects[0]?.id;
    if (projectId) onNewThread(projectId);
    else onAddProject();
  }

  // --- Context menus ---
  let menu = $state<{ x: number; y: number; items: ContextMenuItem[]; align?: "end" } | null>(null);

  function snoozeItems(thread: SidebarThread): ContextMenuItem[] {
    return snoozePresets(new Date()).map((preset) => ({
      id: `snooze:${preset.id}`,
      label: preset.label,
      detail: preset.whenLabel,
      onSelect: () => snooze(thread, preset.snoozedUntil),
    }));
  }

  function threadMenu(thread: SidebarThread, x: number, y: number) {
    const snoozed = !!thread.snoozedUntil && Date.parse(thread.snoozedUntil) > Date.now() && !thread.waiting;
    menu = {
      x,
      y,
      items: [
        thread.pinnedAt
          ? { id: "unpin", label: "Unpin thread", Icon: PinOff, onSelect: () => pin(thread, false) }
          : { id: "pin", label: "Pin thread", Icon: Pin, onSelect: () => pin(thread, true) },
        thread.settledAt
          ? { id: "unsettle", label: "Un-settle thread", Icon: CircleCheck, onSelect: () => settle(thread, false) }
          : { id: "settle", label: "Settle thread", Icon: CircleCheck, onSelect: () => settle(thread, true) },
        snoozed
          ? { id: "unsnooze", label: "Wake thread", Icon: Clock, onSelect: () => snooze(thread, null) }
          : { id: "snooze", label: "Snooze", Icon: Clock, disabled: thread.waiting, children: snoozeItems(thread) },
        { id: "rename", label: "Rename thread", Icon: Pencil, separatorBefore: true, onSelect: () => (renaming = thread.id) },
        {
          id: "mark-unread",
          label: "Mark unread",
          Icon: MailOpen,
          disabled: !thread.turnEndedAt || isUnread(thread),
          onSelect: () => send({ type: "mark_read", threadId: thread.id, read: false }),
        },
        {
          id: "copy",
          label: "Copy",
          Icon: Copy,
          separatorBefore: true,
          children: [
            { id: "copy-path", label: "Path", Icon: Folder, onSelect: () => void navigator.clipboard.writeText(thread.project.dir) },
            { id: "copy-thread-id", label: "Thread ID", Icon: Hash, onSelect: () => void navigator.clipboard.writeText(thread.id) },
          ],
        },
        { id: "project-settings", label: "Project settings", Icon: Settings, onSelect: () => onProjectSettings(thread.project.id) },
        thread.archived
          ? { id: "unarchive", label: "Unarchive thread", Icon: ArchiveRestore, separatorBefore: true, onSelect: () => onArchiveThread(thread.id, false) }
          : {
              id: "archive",
              label: "Archive thread",
              Icon: Archive,
              separatorBefore: true,
              disabled: thread.busy,
              onSelect: () => onArchiveThread(thread.id, true),
            },
        { id: "delete", label: "Delete", Icon: Trash, destructive: true, onSelect: () => deleteThread(thread) },
      ],
    };
  }

  function projectMenu(project: ProjectSummary, x: number, y: number) {
    const update = (changes: { pinned?: boolean; archived?: boolean }) => send({ type: "update_project", projectId: project.id, ...changes });
    menu = {
      x,
      y,
      items: [
        { id: "new-thread", label: "New thread", Icon: SquarePen, onSelect: () => onNewThread(project.id) },
        { id: "copy-path", label: "Copy path", Icon: Copy, onSelect: () => void navigator.clipboard.writeText(project.dir) },
        project.pinned
          ? { id: "unpin", label: "Unpin project", Icon: PinOff, onSelect: () => update({ pinned: false }) }
          : { id: "pin", label: "Pin project", Icon: Pin, onSelect: () => update({ pinned: true }) },
        ...(project.link && !project.open
          ? [{ id: "open-in-resolve", label: "Open in Resolve", Icon: MonitorUp, onSelect: () => onOpenInResolve(project.id) }]
          : []),
        project.archived
          ? { id: "unarchive", label: "Unarchive project", Icon: ArchiveRestore, onSelect: () => update({ archived: false }) }
          : { id: "archive", label: "Archive project", Icon: Archive, onSelect: () => update({ archived: true }) },
        { id: "project-settings", label: "Project settings", Icon: Settings, onSelect: () => onProjectSettings(project.id) },
      ],
    };
  }

  // --- Unread: the open thread is read while this window is in front, as T3 Code records visits ---
  let visible = $state(document.visibilityState === "visible");
  $effect(() => {
    if (!current || !visible) return;
    if (!current.lastReadAt || isUnread(current) || isWoke(current, nowMs)) {
      send({ type: "mark_read", threadId: current.id, read: true });
    }
  });

  // --- Keyboard shortcuts: T3 Code's defaults ---
  const isMac = /Mac|iPhone|iPad/.test(navigator.platform);
  let jumpHints = $state(false);
  let hintTimer: ReturnType<typeof setTimeout> | undefined;
  const jumpLabel = (index: number) => (index < 9 ? (isMac ? `⌘${index + 1}` : `Ctrl+${index + 1}`) : null);

  function onKeydown(event: KeyboardEvent) {
    const mod = isMac ? event.metaKey : event.ctrlKey;
    if (event.key === (isMac ? "Meta" : "Control")) {
      clearTimeout(hintTimer);
      hintTimer = setTimeout(() => (jumpHints = true), 200);
      return;
    }
    if (!mod || event.altKey || document.querySelector("dialog[open]")) return;
    if (!event.shiftKey && event.key.toLowerCase() === "n") {
      event.preventDefault();
      newThread();
    } else if (!event.shiftKey && /^[1-9]$/.test(event.key)) {
      const thread = lists.order[Number(event.key) - 1];
      if (!thread) return;
      event.preventDefault();
      onSelect(thread.id);
    } else if (event.shiftKey && (event.code === "BracketLeft" || event.code === "BracketRight") && current) {
      const index = lists.order.findIndex((thread) => thread.id === current.id);
      const next = lists.order[index + (event.code === "BracketLeft" ? -1 : 1)];
      if (index === -1 || !next) return;
      event.preventDefault();
      onSelect(next.id);
    } else if (event.shiftKey && event.code === "KeyS" && current) {
      event.preventDefault();
      settle(current, !current.settledAt);
    } else if (event.shiftKey && event.code === "KeyP" && current) {
      event.preventDefault();
      pin(current, !current.pinnedAt);
    }
  }

  function hideHints() {
    clearTimeout(hintTimer);
    jumpHints = false;
  }

  const jumpIndex = $derived(new Map(lists.order.slice(0, 9).map((thread, index) => [thread.id, index])));
  const hintFor = (thread: SidebarThread) => {
    const index = jumpIndex.get(thread.id);
    return jumpHints && index !== undefined ? jumpLabel(index) : null;
  };

  // The project open in Resolve when no Linker-Edit project has it yet.
  const unlinkedOpen = $derived(
    resolve.state === "connected" && resolve.project && !projects.some((project) => project.open) ? resolve.project : null,
  );
</script>

<svelte:window
  onkeydown={onKeydown}
  onkeyup={(event) => event.key === (isMac ? "Meta" : "Control") && hideHints()}
  onblur={hideHints}
/>
<svelte:document onvisibilitychange={() => (visible = document.visibilityState === "visible")} />

{#snippet row(thread: SidebarThread, variant: "card" | "folder" | "slim", action: "settle" | "unsettle" | "unsnooze")}
  <SidebarThreadRow
    {thread}
    {variant}
    {action}
    {nowMs}
    active={thread.id === threadId}
    renaming={renaming === thread.id}
    jumpLabel={hintFor(thread)}
    onSelect={() => onSelect(thread.id)}
    onStartRename={() => (renaming = thread.id)}
    onRename={(title) => {
      renaming = null;
      if (title && title !== thread.title) onRenameThread(thread.id, title);
    }}
    onSettle={() => settle(thread, true)}
    onUnsettle={() => settle(thread, false)}
    onUnsnooze={() => snooze(thread, null)}
    onUnpin={() => pin(thread, false)}
    onSnoozeMenu={(x, y) => (menu = { x, y, items: snoozeItems(thread), align: "end" })}
    onAcknowledgeWoke={() => send({ type: "mark_read", threadId: thread.id, read: true })}
    onContextMenu={(x, y) => threadMenu(thread, x, y)}
  />
{/snippet}

<aside class="flex w-64 shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground" data-testid="sidebar">
  <div class="title-bar title-bar-start flex h-13 shrink-0 items-center gap-2 px-4">
    <LinkerLogo class="size-5 text-foreground" />
    <span class="text-sm font-medium tracking-tight">Linker-Edit</span>
    {#if appUpdate}
      <button
        type="button"
        class="ml-auto rounded px-1 text-2xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="Check for updates"
        onclick={checkForUpdates}
      >
        {version === "0.0.0" ? "dev" : releaseName(version)}
      </button>
    {:else}
      <span class="ml-auto text-2xs text-muted-foreground">{version === "0.0.0" ? "dev" : releaseName(version)}</span>
    {/if}
  </div>

  <ComputerPicker {computer} {remote} {resolveHere} {onRemoteAccess} {onResolveHere} />

  <div class="relative z-[1] flex w-full min-w-0 flex-col p-(--sidebar-content-inset)">
    <SidebarSearch
      bind:query
      bind:input={searchInput}
      hasProjects={projects.length > 0}
      resultCount={results.length}
      {activeResult}
      onKeydown={onSearchKeydown}
      {onAddProject}
      onNewThread={newThread}
    />
    {#if unlinkedOpen}
      <button
        type="button"
        class="mt-1 flex h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left text-xs text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
        title="Resolve has this project open; add it to Linker-Edit"
        onclick={onAddOpenProject}
      >
        <Plus class="size-4 shrink-0 text-(--sidebar-icon-color)" />
        <span class="min-w-0 flex-1 truncate">Add <span class="font-medium text-sidebar-foreground">{unlinkedOpen}</span>, open in Resolve</span>
      </button>
    {/if}
  </div>

  <nav class="flex min-h-0 flex-1 flex-col overflow-y-auto px-(--sidebar-content-inset) pb-(--sidebar-content-inset)" aria-label="Threads">
    {#if searching}
      {#if results.length > 0}
        <ul id="sidebar-thread-search-results" role="listbox" aria-label="Thread search results" class="flex flex-col gap-px">
          {#each results as thread, index (thread.id)}
            <SidebarSearchResult
              {thread}
              {index}
              {nowMs}
              highlighted={index === activeResult}
              active={thread.id === threadId}
              onHighlight={() => (activeResult = index)}
              onSelect={() => openResult(thread)}
              onContextMenu={(x, y) => threadMenu(thread, x, y)}
            />
          {/each}
        </ul>
      {:else}
        <p role="status" class="px-2 py-6 text-center text-xs text-sidebar-muted-foreground">No threads found</p>
      {/if}
    {:else if projects.length === 0}
      <div class="flex flex-col items-center gap-2 px-2 py-6 text-center text-xs text-muted-foreground/60">
        <span>No projects yet</span>
        <button
          type="button"
          onclick={onAddProject}
          class="inline-flex cursor-pointer items-center gap-1.5 rounded-md border border-sidebar-border px-2.5 py-1 text-2xs font-medium text-sidebar-muted-foreground transition-colors hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
        >
          <Plus class="-mx-0.5 size-3" />
          Add project
        </button>
      </div>
    {:else}
      <ul role="presentation" class="relative flex flex-1 flex-col gap-px">
        {#each lists.pinned as thread (thread.id)}
          {@render row(thread, "card", "settle")}
        {/each}
        {#if lists.pinned.length > 0 && lists.folders.length > 0}
          <li class="h-1 list-none" aria-hidden="true"></li>
        {/if}

        {#each lists.folders as folder (folder.project.id)}
          {@const isCollapsed = collapsed.has(folder.project.id)}
          {@const shown = folderRows(folder)}
          <SidebarProject
            project={folder.project}
            collapsed={isCollapsed}
            hiddenThreads={isCollapsed ? folder.threads : []}
            onToggle={() => toggleFolder(folder.project.id)}
            onNewThread={() => onNewThread(folder.project.id)}
            onContextMenu={(x, y) => projectMenu(folder.project, x, y)}
          />
          {#if !isCollapsed}
            {#each shown.rows as thread (thread.id)}
              {@render row(thread, "folder", "settle")}
            {/each}
            {#if shown.hidden > 0 || (expandedFolders.has(folder.project.id) && folder.threads.length > FOLDER_PREVIEW_COUNT)}
              <li class="list-none">
                <button
                  type="button"
                  data-testid="sidebar-folder-more"
                  onclick={() => toggleShowAll(folder.project.id)}
                  class="flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-md px-2.5 ps-9 text-left text-sm text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
                >
                  {#if shown.hidden > 0}
                    <ChevronDown aria-hidden="true" class="size-4 shrink-0" />
                    Show {shown.hidden} more
                  {:else}
                    <ChevronUp aria-hidden="true" class="size-4 shrink-0" />
                    Show less
                  {/if}
                </button>
              </li>
            {/if}
          {/if}
        {/each}

        {#if lists.snoozed.length > 0}
          <SidebarShelfHeader
            class="mt-auto"
            label="Snoozed"
            count={lists.snoozed.length}
            expanded={snoozedExpanded}
            onToggle={() => store(SNOOZED_KEY, (snoozedExpanded = !snoozedExpanded))}
          />
          {#if snoozedExpanded}
            {#each lists.snoozed as thread (thread.id)}
              {@render row(thread, "slim", "unsnooze")}
            {/each}
          {/if}
        {/if}

        {#if lists.settled.length > 0}
          <SidebarShelfHeader
            class={lists.snoozed.length === 0 ? "mt-auto" : ""}
            label="Settled"
            count={lists.settled.length}
            expanded={settledExpanded}
            onToggle={() => store(SETTLED_KEY, (settledExpanded = !settledExpanded))}
          />
          {#if settledExpanded}
            {#each lists.settled.slice(0, settledShown) as thread (thread.id)}
              {@render row(thread, "slim", "unsettle")}
            {/each}
            {#if lists.settled.length > settledShown}
              <li class="list-none">
                <button
                  type="button"
                  onclick={() => (settledShown += SETTLED_PAGE_COUNT)}
                  class="flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-md px-2.5 text-left text-sm text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
                >
                  <ChevronDown aria-hidden="true" class="size-4 shrink-0" />
                  Show {Math.min(lists.settled.length - settledShown, SETTLED_PAGE_COUNT)} more
                </button>
              </li>
            {/if}
          {/if}
        {/if}
      </ul>
    {/if}
  </nav>

  <SidebarFooter
    {resolve}
    {resolveHere}
    {update}
    {appUpdate}
    {transcription}
    {computer}
    {linker}
    {asked}
    {onRefreshResolve}
    {onResolveHere}
    {onSettings}
    {onLinkerCancel}
    {onLinkerSignOut}
    {onTranscriptionSetup}
    {onTranscriptionCancel}
    onCheckAppUpdate={checkForUpdates}
    {onInstallAppUpdate}
    {onCancelAppUpdate}
  />
</aside>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} align={menu.align} onClose={() => (menu = null)} />
{/if}
