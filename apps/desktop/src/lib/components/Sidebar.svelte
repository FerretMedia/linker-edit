<script lang="ts" module>
  import { createWorkingTracker } from "#lib/sidebar.ts";

  // Kept for the page's life, like T3 Code's inboxReturns, so a thread's grace on the Working shelf outlives a remount.
  const workingTracker = createWorkingTracker();
</script>

<script lang="ts">
  // A copy of T3 Code's sidebar (apps/web/src/components/Sidebar.tsx and sidebar/*; MIT, see NOTICE): one inbox of
  // every project's threads in shelves, Pinned, Active, Working, Snoozed and Settled, scoped by the Projects button.
  // Its "boring" layout, a folder per project, is a setting. Linker-Edit's own parts (Resolve, transcription,
  // updates, Linker account) stay around it. Like T3 Code with several environments, it lists every machine's
  // projects together, and says which machine each is on when there's more than one.
  import Archive from "@lucide/svelte/icons/archive";
  import ArchiveRestore from "@lucide/svelte/icons/archive-arrow-up";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import ChevronUp from "@lucide/svelte/icons/chevron-up";
  import CircleCheck from "@lucide/svelte/icons/circle-check";
  import Clock from "@lucide/svelte/icons/clock";
  import Copy from "@lucide/svelte/icons/copy";
  import Folder from "@lucide/svelte/icons/folder";
  import Hash from "@lucide/svelte/icons/hash";
  import MailOpen from "@lucide/svelte/icons/mail-open";
  import MonitorUp from "@lucide/svelte/icons/monitor-up";
  import Pencil from "@lucide/svelte/icons/pencil";
  import Pin from "@lucide/svelte/icons/pin";
  import PinOff from "@lucide/svelte/icons/pin-off";
  import Plus from "@lucide/svelte/icons/plus";
  import Settings from "@lucide/svelte/icons/settings";
  import SquarePen from "@lucide/svelte/icons/square-pen";
  import Trash from "@lucide/svelte/icons/trash";
  import type { AppMeta, ClientMessage } from "#shared/protocol.ts";
  import { releaseName } from "#shared/release-name.ts";
  import { untrack } from "svelte";
  import { scopedKey } from "#lib/machines.ts";
  import {
    FOLDER_PREVIEW_COUNT,
    SETTLED_INITIAL_COUNT,
    SETTLED_PAGE_COUNT,
    isUnread,
    isWoke,
    listedThreads,
    nextThreadAfter,
    nextWake,
    partitionSidebar,
    scopedProjects,
    searchThreads,
    snoozePresets,
    sortProjects,
    type SidebarProject as Project,
    type SidebarThread,
  } from "#lib/sidebar.ts";
  import { sidebarPrefs } from "#lib/sidebarPrefs.svelte.ts";
  import LinkerLogo from "./LinkerLogo.svelte";
  import ContextMenu, { type ContextMenuItem } from "./sidebar/ContextMenu.svelte";
  import SidebarFooter from "./sidebar/SidebarFooter.svelte";
  import SidebarProject from "./sidebar/SidebarProject.svelte";
  import SidebarProjectScope from "./sidebar/SidebarProjectScope.svelte";
  import SidebarSearch from "./sidebar/SidebarSearch.svelte";
  import SidebarSearchResult from "./sidebar/SidebarSearchResult.svelte";
  import SidebarShelfHeader from "./sidebar/SidebarShelfHeader.svelte";
  import SidebarThreadRow from "./sidebar/SidebarThreadRow.svelte";

  let {
    meta,
    machines,
    projects,
    loading,
    selected,
    send,
    onSelect,
    onNewThread,
    onAddProject,
    onProjectSettings,
    onSettings,
    onBrands,
    onSignOut = null,
  }: {
    /** The open machine's, for the foot: its Resolve, updates and Linker account. Null while none is connected. */
    meta: { machineId: string; app: AppMeta } | null;
    /** Every machine, this computer first; their names show only when there's more than one. */
    machines: { id: string; name: string; status: "connecting" | "connected" | "offline" }[];
    /** Every machine's projects together. */
    projects: Project[];
    /** A machine hasn't said yet which projects it has. */
    loading: boolean;
    /** The open thread. */
    selected: { machineId: string; threadId: string } | null;
    /** To the machine a thread or project is on: pinning, settling, renaming, project icons… */
    send: (machineId: string, message: ClientMessage) => void;
    onSelect: (machineId: string, threadId: string) => void;
    onNewThread: (machineId: string, projectId: string) => void;
    onAddProject: () => void;
    onProjectSettings: (machineId: string, projectId: string) => void;
    onSettings: () => void;
    onBrands: () => void;
    /** On a web address: signs this browser out of Linker. */
    onSignOut?: (() => void) | null;
  } = $props();

  const multi = $derived(machines.length > 1);
  const machineNames = $derived(new Map(machines.map((machine) => [machine.id, machine.name])));
  /** The machine's name on its rows, only when there's more than one. */
  const machineOf = (project: Project) => (multi ? (machineNames.get(project.machineId) ?? null) : null);
  const status = $derived(new Map(machines.map((machine) => [machine.id, machine.status])));
  const selectedKey = $derived(selected ? scopedKey(selected.machineId, selected.threadId) : null);
  const app = $derived(meta?.app ?? null);
  /** To the open machine, for the foot's buttons. */
  function sendHere(message: ClientMessage) {
    if (meta) send(meta.machineId, message);
  }

  // The desktop app updates itself (appUpdate); `update` is the npm release, for `npx linker-edit`.
  // After "Check for updates", the card also says when there's nothing new, for a few seconds.
  let asked = $state(false);
  $effect(() => {
    if (!asked || app?.appUpdate?.status === "checking") return;
    const timer = setTimeout(() => (asked = false), 6000);
    return () => clearTimeout(timer);
  });

  function checkForUpdates() {
    asked = true;
    sendHere({ type: "check_app_update" });
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

  const current = $derived.by((): SidebarThread | null => {
    if (!selected) return null;
    for (const project of projects) {
      if (project.machineId !== selected.machineId) continue;
      const thread = project.threads.find((entry) => entry.id === selected.threadId);
      if (thread) return { ...thread, project, key: scopedKey(project.machineId, thread.id) };
    }
    return null;
  });

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
  const WORKING_KEY = "linker-edit.sidebar.working-expanded";
  const SCOPE_KEY = "linker-edit.sidebar.project-scope";
  let collapsed = $state(new Set<string>(stored<string[]>(COLLAPSED_KEY, [])));
  // Every shelf starts closed, as in T3 Code; the open thread shows in a closed one all the same.
  let settledExpanded = $state(stored(SETTLED_KEY, false));
  let snoozedExpanded = $state(stored(SNOOZED_KEY, false));
  let workingExpanded = $state(stored(WORKING_KEY, false));
  let expandedFolders = $state(new Set<string>());

  // --- The project scope: which projects' threads are listed, T3 Code's scopedProjectKeys ---
  let savedScope = $state(stored<string[]>(SCOPE_KEY, []));
  /** The picked projects still here and not archived; empty lists every project. */
  const scope = $derived(savedScope.filter((key) => projects.some((project) => project.key === key && !project.archived)));
  const scopeSet = $derived(scope.length > 0 ? new Set(scope) : null);
  function setScope(keys: string[]) {
    savedScope = keys;
    store(SCOPE_KEY, keys);
  }
  // Opening a thread outside the scope, as a new one in another project, widens it to every project, as T3 Code's
  // route reveal does. Picking a scope while a thread is open doesn't.
  let revealed: string | null = null;
  $effect(() => {
    if (!current || current.key === revealed) return;
    revealed = current.key;
    const projectKey = current.project.key;
    untrack(() => {
      if (scopeSet && !scopeSet.has(projectKey)) setScope([]);
    });
  });

  const lists = $derived(
    partitionSidebar(projects, nowMs, {
      scope: scopeSet,
      folders: sidebarPrefs.folders,
      working: sidebarPrefs.folders ? undefined : workingTracker.observe(listedThreads(projects), Date.now()),
    }),
  );
  // Looks again when a thread's grace on the Working shelf ends.
  $effect(() => {
    void lists;
    const deadline = workingTracker.nextDeadline();
    if (deadline === null) return;
    const timer = setTimeout(() => (nowMs = Date.now()), Math.max(0, deadline - Date.now()) + 20);
    return () => clearTimeout(timer);
  });

  // The Settled shelf's page starts over when the scope changes, as in T3 Code.
  let settledPage = $state({ scope: "", count: SETTLED_INITIAL_COUNT });
  const settledShown = $derived(settledPage.scope === scope.join("\n") ? settledPage.count : SETTLED_INITIAL_COUNT);
  const showMoreSettled = () => (settledPage = { scope: scope.join("\n"), count: settledShown + SETTLED_PAGE_COUNT });

  /** A closed shelf still shows the open thread. */
  const onShelf = (threads: SidebarThread[], expanded: boolean) => (expanded ? threads : threads.filter((thread) => thread.key === selectedKey));
  const shownWorking = $derived(onShelf(lists.working, workingExpanded));
  const shownSnoozed = $derived(onShelf(lists.snoozed, snoozedExpanded));
  const shownSettled = $derived.by(() => {
    const page = lists.settled.filter((thread, index) => index < settledShown || thread.key === selectedKey);
    return onShelf(page, settledExpanded);
  });
  const hiddenSettled = $derived(settledExpanded ? lists.settled.length - shownSettled.length : 0);
  const shelvesFirst = $derived(lists.working.length > 0 ? "working" : lists.snoozed.length > 0 ? "snoozed" : "settled");

  function toggleFolder(projectKey: string) {
    const next = new Set(collapsed);
    if (!next.delete(projectKey)) next.add(projectKey);
    collapsed = next;
    store(COLLAPSED_KEY, [...next]);
  }

  function toggleShowAll(projectKey: string) {
    const next = new Set(expandedFolders);
    if (!next.delete(projectKey)) next.add(projectKey);
    expandedFolders = next;
  }

  /** A folder's first six, and the open thread when it's further down. */
  function folderRows(folder: { project: Project; threads: SidebarThread[] }) {
    if (expandedFolders.has(folder.project.key) || folder.threads.length <= FOLDER_PREVIEW_COUNT) return { rows: folder.threads, hidden: 0 };
    const rows = folder.threads.filter((thread, index) => index < FOLDER_PREVIEW_COUNT || thread.key === selectedKey);
    return { rows, hidden: folder.threads.length - rows.length };
  }

  // --- Search ---
  let query = $state("");
  let searchInput: HTMLInputElement | undefined = $state();
  let activeResult = $state(0);
  const searching = $derived(query.trim().length > 0);
  // Searches the scoped projects, as T3 Code does.
  const results = $derived(searching ? searchThreads(scopedProjects(projects, scopeSet), query) : []);
  $effect(() => {
    void query;
    activeResult = 0;
  });

  /** An offline machine's threads show as they were last, but don't open. */
  function open(thread: SidebarThread) {
    if (!thread.project.offline) onSelect(thread.project.machineId, thread.id);
  }

  function openResult(thread: SidebarThread) {
    query = "";
    open(thread);
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
    if (thread.key !== selectedKey) return;
    const next = nextThreadAfter(
      lists.order.filter((entry) => !entry.project.offline || entry.key === thread.key),
      thread.key,
    );
    if (next) onSelect(next.project.machineId, next.id);
    else onNewThread(thread.project.machineId, thread.project.id);
  }

  /** To the machine the thread is on. */
  const sendFor = (thread: SidebarThread, message: ClientMessage) => send(thread.project.machineId, message);

  function settle(thread: SidebarThread, settled: boolean) {
    if (settled) moveOnFrom(thread);
    sendFor(thread, { type: "settle_thread", threadId: thread.id, settled });
  }

  function snooze(thread: SidebarThread, until: string | null) {
    if (until) moveOnFrom(thread);
    sendFor(thread, { type: "snooze_thread", threadId: thread.id, until });
  }

  const pin = (thread: SidebarThread, pinned: boolean) => sendFor(thread, { type: "pin_thread", threadId: thread.id, pinned });
  const archive = (thread: SidebarThread, archived: boolean) => sendFor(thread, { type: "archive_thread", threadId: thread.id, archived });

  function deleteThread(thread: SidebarThread) {
    if (confirm(`Delete "${thread.title}"? The chat is gone for good; Resolve and the edit history keep what it changed.`)) {
      sendFor(thread, { type: "delete_thread", threadId: thread.id });
    }
  }

  /** In the one project the scope lists, else the open thread's, else the first one listed on a machine that's connected. */
  function newThread() {
    const scoped = scope.length === 1 ? projects.find((project) => project.key === scope[0]) : undefined;
    const project = [scoped, current?.project, ...lists.order.map((thread) => thread.project), ...sortProjects(projects)].find(
      (entry) => entry && !entry.offline,
    );
    if (project) onNewThread(project.machineId, project.id);
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
    if (thread.project.offline) return;
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
        { id: "rename", label: "Rename thread", Icon: Pencil, separatorBefore: true, onSelect: () => (renaming = thread.key) },
        {
          id: "mark-unread",
          label: "Mark unread",
          Icon: MailOpen,
          disabled: !thread.turnEndedAt || isUnread(thread),
          onSelect: () => sendFor(thread, { type: "mark_read", threadId: thread.id, read: false }),
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
        {
          id: "project-settings",
          label: "Project settings",
          Icon: Settings,
          onSelect: () => onProjectSettings(thread.project.machineId, thread.project.id),
        },
        thread.archived
          ? { id: "unarchive", label: "Unarchive thread", Icon: ArchiveRestore, separatorBefore: true, onSelect: () => archive(thread, false) }
          : {
              id: "archive",
              label: "Archive thread",
              Icon: Archive,
              separatorBefore: true,
              disabled: thread.busy,
              onSelect: () => archive(thread, true),
            },
        { id: "delete", label: "Delete", Icon: Trash, destructive: true, onSelect: () => deleteThread(thread) },
      ],
    };
  }

  function projectMenu(project: Project, x: number, y: number) {
    if (project.offline) return;
    const update = (changes: { pinned?: boolean; archived?: boolean }) =>
      send(project.machineId, { type: "update_project", projectId: project.id, ...changes });
    menu = {
      x,
      y,
      items: [
        { id: "new-thread", label: "New thread", Icon: SquarePen, onSelect: () => onNewThread(project.machineId, project.id) },
        { id: "copy-path", label: "Copy path", Icon: Copy, onSelect: () => void navigator.clipboard.writeText(project.dir) },
        project.pinned
          ? { id: "unpin", label: "Unpin project", Icon: PinOff, onSelect: () => update({ pinned: false }) }
          : { id: "pin", label: "Pin project", Icon: Pin, onSelect: () => update({ pinned: true }) },
        ...(project.link && !project.open
          ? [{ id: "open-in-resolve", label: "Open in Resolve", Icon: MonitorUp, onSelect: () => send(project.machineId, { type: "open_in_resolve", projectId: project.id }) }]
          : []),
        project.archived
          ? { id: "unarchive", label: "Unarchive project", Icon: ArchiveRestore, onSelect: () => update({ archived: false }) }
          : { id: "archive", label: "Archive project", Icon: Archive, onSelect: () => update({ archived: true }) },
        { id: "project-settings", label: "Project settings", Icon: Settings, onSelect: () => onProjectSettings(project.machineId, project.id) },
      ],
    };
  }

  // --- Unread: the open thread is read while this window is in front, as T3 Code records visits ---
  let visible = $state(document.visibilityState === "visible");
  $effect(() => {
    if (!current || !visible) return;
    if (!current.lastReadAt || isUnread(current) || isWoke(current, nowMs)) {
      sendFor(current, { type: "mark_read", threadId: current.id, read: true });
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
      open(thread);
    } else if (event.shiftKey && (event.code === "BracketLeft" || event.code === "BracketRight") && current) {
      const index = lists.order.findIndex((thread) => thread.key === current.key);
      const next = lists.order[index + (event.code === "BracketLeft" ? -1 : 1)];
      if (index === -1 || !next) return;
      event.preventDefault();
      open(next);
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

  const jumpIndex = $derived(new Map(lists.order.slice(0, 9).map((thread, index) => [thread.key, index])));
  const hintFor = (thread: SidebarThread) => {
    const index = jumpIndex.get(thread.key);
    return jumpHints && index !== undefined ? jumpLabel(index) : null;
  };

  // The project open in the open machine's Resolve when none of its Linker-Edit projects has it yet.
  const unlinkedOpen = $derived(
    app?.resolve.state === "connected" && app.resolve.project && !app.projects.some((project) => project.open) ? app.resolve.project : null,
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
    machine={machineOf(thread.project)}
    active={thread.key === selectedKey}
    renaming={renaming === thread.key}
    jumpLabel={hintFor(thread)}
    onSelect={() => open(thread)}
    onStartRename={() => {
      if (!thread.project.offline) renaming = thread.key;
    }}
    onRename={(title) => {
      renaming = null;
      if (title && title !== thread.title) sendFor(thread, { type: "rename_thread", threadId: thread.id, title });
    }}
    onSettle={() => settle(thread, true)}
    onUnsettle={() => settle(thread, false)}
    onUnsnooze={() => snooze(thread, null)}
    onUnpin={() => pin(thread, false)}
    onSnoozeMenu={(x, y) => (menu = { x, y, items: snoozeItems(thread), align: "end" })}
    onAcknowledgeWoke={() => sendFor(thread, { type: "mark_read", threadId: thread.id, read: true })}
    onContextMenu={(x, y) => threadMenu(thread, x, y)}
  />
{/snippet}

<aside class="flex w-64 shrink-0 flex-col border-r bg-sidebar text-sidebar-foreground" data-testid="sidebar">
  <div class="title-bar title-bar-start flex h-13 shrink-0 items-center gap-2 px-4">
    <LinkerLogo class="size-5 text-foreground" />
    <span class="text-sm font-medium tracking-tight">Linker-Edit</span>
    {#if app?.appUpdate}
      <button
        type="button"
        class="ml-auto rounded px-1 text-2xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="Check for updates"
        onclick={checkForUpdates}
      >
        {app.version === "0.0.0" ? "dev" : releaseName(app.version)}
      </button>
    {:else if app}
      <span class="ml-auto text-2xs text-muted-foreground">{app.version === "0.0.0" ? "dev" : releaseName(app.version)}</span>
    {/if}
  </div>


  <div class="relative z-[1] flex w-full min-w-0 flex-col p-(--sidebar-content-inset)">
    <SidebarSearch
      bind:query
      bind:input={searchInput}
      hasProjects={projects.some((project) => !project.offline)}
      resultCount={results.length}
      {activeResult}
      onKeydown={onSearchKeydown}
      onNewThread={newThread}
    >
      {#snippet projectScope()}
        {#if projects.length > 0}
          <SidebarProjectScope
            projects={sortProjects(projects)}
            {scope}
            {machineOf}
            machineStatus={(project) => status.get(project.machineId) ?? "offline"}
            onScope={setScope}
            {onAddProject}
            onPin={(project) => send(project.machineId, { type: "update_project", projectId: project.id, pinned: !project.pinned })}
            onProjectSettings={(project) => onProjectSettings(project.machineId, project.id)}
            onContextMenu={(project, x, y) => projectMenu(project, x, y)}
          />
        {/if}
      {/snippet}
    </SidebarSearch>
    {#if unlinkedOpen}
      <button
        type="button"
        class="mt-1 flex h-8 w-full cursor-pointer items-center gap-2 rounded-md px-2 text-left text-xs text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
        title="Resolve has this project open; add it to Linker-Edit"
        onclick={() => sendHere({ type: "add_project", source: { kind: "open" } })}
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
          {#each results as thread, index (thread.key)}
            <SidebarSearchResult
              {thread}
              {index}
              {nowMs}
              machine={machineOf(thread.project)}
              highlighted={index === activeResult}
              active={thread.key === selectedKey}
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
      {#if !loading}
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
      {/if}
    {:else}
      {@const scopedNames = projects.filter((project) => scopeSet?.has(project.key)).map((project) => project.name)}
      {#if lists.order.length === 0 && lists.snoozed.length === 0 && (lists.folders ?? []).length === 0 && (lists.settled.length === 0 || scopedNames.length > 0)}
        <!-- A scope can hide every active thread; it says so even when its project still has settled ones. -->
        <div class="flex flex-col items-center gap-2 px-2 py-6 text-center text-xs text-muted-foreground/60" data-testid="sidebar-empty">
          {#if scopedNames.length > 0}
            <span>
              {#if lists.settled.length > 0}
                {scopedNames.length > 1 ? "No active threads in these projects" : `No active threads in ${scopedNames[0]}`}
              {:else}
                {scopedNames.length > 1 ? "No threads in these projects yet" : `No threads in ${scopedNames[0]} yet`}
              {/if}
            </span>
            <button
              type="button"
              onclick={() => setScope([])}
              class="inline-flex cursor-pointer items-center gap-1.5 rounded-md border border-sidebar-border px-2.5 py-1 text-2xs font-medium text-sidebar-muted-foreground transition-colors hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
            >
              Show all projects
            </button>
          {:else}
            <span>No threads yet</span>
          {/if}
        </div>
      {/if}
      <ul role="presentation" class="relative flex flex-1 flex-col gap-px">
        {#each lists.pinned as thread (thread.key)}
          {@render row(thread, "card", "settle")}
        {/each}
        {#if lists.pinned.length > 0 && (lists.active.length > 0 || (lists.folders ?? []).length > 0)}
          <li class="h-1 list-none" aria-hidden="true"></li>
        {/if}

        {#each lists.active as thread (thread.key)}
          {@render row(thread, "card", "settle")}
        {/each}

        {#each lists.folders ?? [] as folder (folder.project.key)}
          {@const isCollapsed = collapsed.has(folder.project.key)}
          {@const shown = folderRows(folder)}
          <SidebarProject
            project={folder.project}
            machine={machineOf(folder.project)}
            machineStatus={status.get(folder.project.machineId) ?? "offline"}
            collapsed={isCollapsed}
            hiddenThreads={isCollapsed ? folder.threads.filter((thread) => thread.key !== selectedKey) : []}
            onToggle={() => toggleFolder(folder.project.key)}
            onNewThread={() => onNewThread(folder.project.machineId, folder.project.id)}
            onContextMenu={(x, y) => projectMenu(folder.project, x, y)}
          />
          {#if !isCollapsed}
            {#each shown.rows as thread (thread.key)}
              {@render row(thread, "folder", "settle")}
            {/each}
            {#if shown.hidden > 0 || (expandedFolders.has(folder.project.key) && folder.threads.length > FOLDER_PREVIEW_COUNT)}
              <li class="list-none">
                <button
                  type="button"
                  data-testid="sidebar-folder-more"
                  onclick={() => toggleShowAll(folder.project.key)}
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
          {:else}
            <!-- A closed folder still shows the open thread. -->
            {#each folder.threads.filter((thread) => thread.key === selectedKey) as thread (thread.key)}
              {@render row(thread, "folder", "settle")}
            {/each}
          {/if}
        {/each}

        {#if lists.working.length > 0}
          <SidebarShelfHeader
            class="mt-auto"
            label="Working"
            count={lists.working.length}
            expanded={workingExpanded}
            onToggle={() => store(WORKING_KEY, (workingExpanded = !workingExpanded))}
          />
          {#each shownWorking as thread (thread.key)}
            {@render row(thread, "card", "settle")}
          {/each}
        {/if}

        {#if lists.snoozed.length > 0}
          <SidebarShelfHeader
            class={shelvesFirst === "snoozed" ? "mt-auto" : ""}
            label="Snoozed"
            count={lists.snoozed.length}
            expanded={snoozedExpanded}
            onToggle={() => store(SNOOZED_KEY, (snoozedExpanded = !snoozedExpanded))}
          />
          {#each shownSnoozed as thread (thread.key)}
            {@render row(thread, "slim", "unsnooze")}
          {/each}
        {/if}

        {#if lists.settled.length > 0}
          <SidebarShelfHeader
            class={shelvesFirst === "settled" ? "mt-auto" : ""}
            label="Settled"
            count={lists.settled.length}
            expanded={settledExpanded}
            onToggle={() => store(SETTLED_KEY, (settledExpanded = !settledExpanded))}
          />
          {#each shownSettled as thread (thread.key)}
            {@render row(thread, "slim", "unsettle")}
          {/each}
          {#if hiddenSettled > 0}
            <li class="list-none">
              <button
                type="button"
                data-testid="sidebar-settled-more"
                onclick={showMoreSettled}
                class="flex h-9 w-full cursor-pointer items-center gap-2.5 rounded-md px-2.5 text-left text-sm text-sidebar-muted-foreground hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
              >
                <ChevronDown aria-hidden="true" class="size-4 shrink-0" />
                Show {Math.min(hiddenSettled, SETTLED_PAGE_COUNT)} more
              </button>
            </li>
          {/if}
        {/if}
      </ul>
    {/if}
  </nav>

  <SidebarFooter
    {app}
    machine={meta && multi ? (machineNames.get(meta.machineId) ?? null) : null}
    {asked}
    {onSettings}
    {onBrands}
    {onSignOut}
    send={sendHere}
    onCheckAppUpdate={checkForUpdates}
  />
</aside>

{#if menu}
  <ContextMenu x={menu.x} y={menu.y} items={menu.items} align={menu.align} onClose={() => (menu = null)} />
{/if}
