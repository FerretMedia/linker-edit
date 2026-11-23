<script lang="ts">
  import { tick, untrack } from "svelte";
  import CircleAlert from "@lucide/svelte/icons/circle-alert";
  import FileText from "@lucide/svelte/icons/file-text";
  import FolderPlus from "@lucide/svelte/icons/folder-plus";
  import History from "@lucide/svelte/icons/rotate-ccw-clock";
  import Library from "@lucide/svelte/icons/library";
  import Monitor from "@lucide/svelte/icons/monitor";
  import MonitorPlay from "@lucide/svelte/icons/monitor-play";
  import MonitorUp from "@lucide/svelte/icons/monitor-up";
  import Network from "@lucide/svelte/icons/network";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import WifiOff from "@lucide/svelte/icons/wifi-off";
  import X from "@lucide/svelte/icons/x";
  import type { ChatItem, ClientMessage, HistoryEntry, Provider } from "#shared/protocol.ts";
  import AddProjectDialog from "#lib/components/AddProjectDialog.svelte";
  import ApprovalCard from "#lib/components/ApprovalCard.svelte";
  import Composer from "#lib/components/Composer.svelte";
  import HistoryPanel from "#lib/components/HistoryPanel.svelte";
  import ProjectFilePanel from "#lib/components/ProjectFilePanel.svelte";
  import LibraryPage from "#lib/components/LibraryPage.svelte";
  import PreviewPanel from "#lib/components/PreviewPanel.svelte";
  import ProjectSettingsDialog from "#lib/components/ProjectSettingsDialog.svelte";
  import SettingsPage from "#lib/components/SettingsPage.svelte";
  import Sidebar from "#lib/components/Sidebar.svelte";
  import NewVideoStarter from "#lib/components/NewVideoStarter.svelte";
  import ThreadStarter from "#lib/components/ThreadStarter.svelte";
  import ToolRow from "#lib/components/ToolRow.svelte";
  import WorkingIndicator from "#lib/components/WorkingIndicator.svelte";
  import { ServerConnection } from "#lib/connection.svelte.ts";
  import { compactSuggested } from "#lib/context.ts";
  import { Host } from "#lib/host.ts";
  import { provideHost } from "#lib/hostContext.ts";
  import { libraryHash, libraryTabFromHash, type LibraryTab } from "#lib/library.ts";
  import { machines } from "#lib/machines.svelte.ts";
  import { scopedKey } from "#lib/machines.ts";
  import { renderMarkdown } from "#lib/markdown.ts";
  import type { PreviewTarget } from "#lib/player.ts";
  import { sectionFromHash, settingsHash, type SettingsSection } from "#lib/settings.ts";
  import { machineProjects } from "#lib/sidebar.ts";
  import { web } from "#lib/web.svelte.ts";

  const SUGGESTIONS = [
    "Describe the open project and its timelines",
    "What's in the media pool?",
    "Duplicate the current timeline and remove the silences",
  ];

  // The open thread's machine: the main pane, its panels and the library talk to its server, as they did to the one
  // server before. Until one is connected, a connection that never connects stands in.
  const idle = new ServerConnection(new Host());
  const active = $derived(machines.active);
  const server = $derived(active?.server ?? idle);
  provideHost(() => server.host);

  /** To the machine a project or thread is on. */
  const sendTo = (machineId: string, message: ClientMessage) => machines.get(machineId)?.server.send(message);

  // One draft per thread, like T3 Code, so switching threads never sends a draft to the wrong one.
  // Kept in this browser too, so a reload or a restart of the app doesn't lose what was typed.
  const DRAFTS_KEY = "linker-edit:drafts";
  function loadDrafts(): Record<string, string> {
    try {
      const saved = JSON.parse(localStorage.getItem(DRAFTS_KEY) ?? "{}") as unknown;
      return saved && typeof saved === "object" ? (saved as Record<string, string>) : {};
    } catch {
      return {};
    }
  }
  let drafts = $state<Record<string, string>>(loadDrafts());
  $effect(() => {
    const kept = Object.fromEntries(Object.entries(drafts).filter(([, text]) => text.trim()));
    try {
      localStorage.setItem(DRAFTS_KEY, JSON.stringify(kept));
    } catch {
      // Private windows and full storage only cost the drafts after a reload.
    }
  });
  // A thread id is only unique on its machine.
  const draftKey = $derived(active && server.state?.threadId ? scopedKey(active.id, server.state.threadId) : "");
  const draft = $derived(drafts[draftKey] ?? "");
  const setDraft = (value: string, key = draftKey) => (drafts[key] = value);

  $effect(() => {
    for (const entry of machines.list) {
      entry.server.onRefused = (threadId, text) => {
        const key = scopedKey(entry.id, threadId);
        if (!(drafts[key] ?? "").trim()) setDraft(text, key);
      };
    }
  });

  $effect(() => {
    // Remembers the thread the open machine shows, as after a new thread, so a reload opens it again.
    if (active && server.state) machines.follow(active.id, server.state.threadId);
  });

  let dialog = $state<{ kind: "add" } | { kind: "settings"; machineId: string; projectId: string } | null>(null);

  // The settings page replaces the app while open; its address is "#settings/<section>", so Back closes it.
  let settings = $state<SettingsSection | null>(sectionFromHash(location.hash));
  // The library page works the same way, at "#library" or "#library/project".
  let library = $state<LibraryTab | null>(libraryTabFromHash(location.hash));
  // The machine the settings page shows; it starts on the open thread's.
  let settingsMachineId = $state<string | null>(null);
  const settingsMachine = $derived.by(() => {
    const chosen = settingsMachineId ? machines.get(settingsMachineId) : null;
    return chosen?.server.state ? chosen : active;
  });
  // Shown once a machine has settings to show; until then the app stays, as it says it's connecting.
  const settingsApp = $derived(settingsMachine?.server.state ?? null);
  function openSettings(section: SettingsSection = "general") {
    settingsMachineId = active?.id ?? null;
    location.hash = settingsHash(section);
  }
  const openLibrary = (tab: LibraryTab = "universal") => (location.hash = libraryHash(tab));
  function closeSettings() {
    if (history.state?.fromApp) history.back();
    else history.replaceState(null, "", location.pathname + location.search);
    settings = null;
    library = null;
  }
  function showSection(section: SettingsSection) {
    history.replaceState(history.state, "", settingsHash(section));
    settings = section;
  }
  function showLibraryTab(tab: LibraryTab) {
    history.replaceState(history.state, "", libraryHash(tab));
    library = tab;
  }
  function onHashChange() {
    settings = sectionFromHash(location.hash);
    library = libraryTabFromHash(location.hash);
    // Remembers that the app came before, so closing goes back rather than adding another entry.
    if ((settings || library) && !history.state?.fromApp) history.replaceState({ fromApp: true }, "");
  }
  function onKeydown(event: KeyboardEvent) {
    if (event.key === "," && (event.ctrlKey || event.metaKey)) {
      event.preventDefault();
      if (settings) closeSettings();
      else openSettings();
    } else if (event.key === "Escape" && (settings || library) && !document.querySelector("dialog[open]")) {
      closeSettings();
    }
  }
  let composer: ReturnType<typeof Composer> | undefined = $state();
  let scroller: HTMLElement | undefined = $state();
  let stickToBottom = true;
  // One side panel at a time, opened from the header like T3 Code's diff panel.
  let panel = $state<"preview" | "history" | "file" | null>(null);
  const historyOpen = $derived(panel === "history");

  const app = $derived(server.state);
  const project = $derived(app?.projects.find((candidate) => candidate.id === app.projectId) ?? null);
  const thread = $derived(project?.threads.find((candidate) => candidate.id === app?.threadId) ?? null);
  const rendering = $derived(app?.preview.state === "rendering");
  const canSend = $derived(
    server.connected && !!app?.threadId && !app.busy && (!app.blocked || app.blocked.canSend === true) && !rendering && draft.trim().length > 0,
  );
  let now = $state(Date.now());
  // The cache expiry the user switched to sending the full chat for.
  let fullFor = $state<string | null>(null);
  const compactChip = $derived.by(() => {
    if (!app || !compactSuggested(app.provider, app.context, now)) return null;
    return fullFor === app.context?.cacheExpiresAt ? "full" : "compact";
  });

  $effect(() => {
    // Wakes once when Claude's prompt cache expires, to offer compacting; no ticking clock.
    const expires = app?.context?.cacheExpiresAt;
    if (!expires) return;
    const wait = Date.parse(expires) - Date.now();
    if (wait <= 0) {
      now = Date.now();
      return;
    }
    const timer = setTimeout(() => (now = Date.now()), wait + 1000);
    return () => clearTimeout(timer);
  });

  const chatTitle = $derived(thread?.title ?? "New thread");

  // The workspace or series the open project is in, or sets up.
  const collection = $derived(
    project ? (app?.collections.find((candidate) => candidate.projectIds.includes(project.id) || candidate.setupProjectId === project.id) ?? null) : null,
  );
  const startBlocked = $derived(
    !server.connected ? "Not connected." : app?.busy ? "Wait for the agent to finish." : ((app?.blocked?.canSend ? null : app?.blocked?.message) ?? (rendering ? "Wait for the render." : null)),
  );

  const sidebarMachines = $derived(machines.list.map((entry) => ({ id: entry.id, name: entry.name, status: entry.status })));
  const sidebarProjects = $derived(
    machineProjects(machines.list.map((entry) => ({ id: entry.id, projects: entry.projects, offline: entry.status !== "connected" }))),
  );
  /** On a web address, signed in to an account with no computers yet. */
  const noComputers = $derived(web.phase.kind === "app" && machines.loaded && machines.list.length === 0);

  /** The first message of a setup thread, from the folders picked. */
  function start(lines: string[], notes: string) {
    server.send({ type: "send", text: [...lines, ...(notes ? ["", notes] : [])].join("\n") });
    stickToBottom = true;
  }

  /** Feedback on the preview starts with where in the video it is about. */
  function commentAt(place: string) {
    setDraft(`${draft.trim() ? `${draft.trimEnd()}\n` : ""}${place}: `);
    composer?.focus();
  }

  // The side panels show a Resolve project's timelines; a setup thread has none.
  const PANELS = [
    { name: "preview", label: "Preview", Icon: MonitorPlay },
    { name: "history", label: "History", Icon: History },
    { name: "file", label: "Project file", Icon: FileText },
  ] as const;

  // ToolSearch is the agent loading tool schemas: plumbing, not work the user cares about.
  const HIDDEN_TOOLS = new Set(["ToolSearch"]);

  // Consecutive tool calls render as one work group, like T3 Code's work log.
  type Row = { kind: "item"; item: ChatItem } | { kind: "tools"; id: string; items: Extract<ChatItem, { kind: "tool" }>[] };
  const rows = $derived.by(() => {
    const result: Row[] = [];
    for (const item of app?.items ?? []) {
      const last = result.at(-1);
      if (item.kind === "tool") {
        if (HIDDEN_TOOLS.has(item.name)) continue;
        if (last?.kind === "tools") last.items.push(item);
        else result.push({ kind: "tools", id: item.id, items: [item] });
      } else {
        result.push({ kind: "item", item });
      }
    }
    return result;
  });

  // A long thread opens with its newest rows, and "Show earlier" adds more: drawing a 1,000-item chat at once held
  // the window for a quarter of a second. The first row shown stays put while the turn adds rows below it.
  const ROWS_PAGE = 100;
  let firstRow = $state<{ threadId: string | null; index: number }>({ threadId: null, index: 0 });
  $effect.pre(() => {
    const threadId = app?.threadId ?? null;
    if (untrack(() => firstRow.threadId) !== threadId) {
      firstRow = { threadId, index: untrack(() => Math.max(0, rows.length - ROWS_PAGE)) };
    }
  });
  const shownFrom = $derived(
    firstRow.threadId === (app?.threadId ?? null) ? Math.min(firstRow.index, rows.length) : Math.max(0, rows.length - ROWS_PAGE),
  );
  const shownRows = $derived(shownFrom > 0 ? rows.slice(shownFrom) : rows);

  async function showEarlier() {
    if (!scroller) return;
    // Keeps what's on screen where it is; the earlier rows appear above it.
    const fromBottom = scroller.scrollHeight - scroller.scrollTop;
    firstRow = { threadId: app?.threadId ?? null, index: Math.max(0, shownFrom - ROWS_PAGE) };
    await tick();
    scroller.scrollTop = scroller.scrollHeight - fromBottom;
  }

  const followKey = $derived.by(() => {
    const last = app?.items.at(-1);
    const size = last ? ("text" in last ? last.text.length : last.kind === "tool" ? `${last.status}${last.output?.length}` : "") : "";
    return `${app?.items.length}:${size}:${app?.approvals.length}:${app?.busy}`;
  });

  $effect(() => {
    // Keeps the newest output in view unless the user scrolled up to read.
    void followKey;
    if (scroller && stickToBottom) scroller.scrollTop = scroller.scrollHeight;
  });

  $effect(() => {
    // Asks again after a reconnect or a switch to another project; while open, the server pushes every new save.
    if (historyOpen && server.connected && app?.projectId) server.send({ type: "load_history" });
  });

  $effect(() => {
    // While open, the server pushes it again after each turn and each save.
    // Also for an empty thread: a project with no PROJECT.md can start a video from its brief.
    if ((panel === "file" || app?.items.length === 0) && server.connected && app?.projectId) server.send({ type: "load_project_file" });
  });

  /** The thread whose New video form is open. */
  let startingVideo = $state<string | null>(null);
  const projectFile = $derived(server.projectFile && server.projectFile.projectId === app?.projectId ? server.projectFile : null);
  const canStartVideo = $derived(!!project && !project.setup && projectFile?.text.trim() === "");

  const historyEntries = $derived(server.history && server.history.projectId === app?.projectId ? server.history.entries : null);

  function restore(entry: HistoryEntry) {
    const names = entry.timelines.map((timeline) => `"${timeline.name}"`);
    const which = names.length ? `${names.join(", ")} as ${names.length === 1 ? "it was" : "they were"}` : "the timelines as they were";
    setDraft(`Restore ${which} in save ${entry.commit} ("${entry.subject}") with restore_timeline.`);
    composer?.focus();
  }

  function togglePanel(name: "preview" | "history" | "file") {
    panel = panel === name ? null : name;
  }

  // The current timeline until a version is picked from the history; versions belong to one project.
  let previewTarget = $state<{ projectId: string | null; target: PreviewTarget }>({ projectId: null, target: { id: null, commit: null } });
  const currentTarget = $derived(
    previewTarget.projectId === app?.projectId ? previewTarget.target : { id: null, commit: null },
  );
  const resolveBusy = $derived(
    !!app && (app.busy || app.opening !== null || app.applying !== null || app.projects.some((candidate) => candidate.threads.some((entry) => entry.busy))),
  );

  function openPreview(target: PreviewTarget) {
    previewTarget = { projectId: app?.projectId ?? null, target };
    panel = "preview";
  }

  function onWindowFocus() {
    // The user may have switched projects in Resolve meanwhile.
    server.send({ type: "refresh_resolve" });
  }

  function onScroll() {
    if (!scroller) return;
    stickToBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
  }

  function send() {
    if (!canSend) return;
    server.send({ type: "send", text: draft.trim(), compactFirst: compactChip === "compact" });
    setDraft("");
    stickToBottom = true;
  }

  function selectProvider(provider: Provider) {
    server.send({ type: "set_provider", provider });
  }

  function useSuggestion(text: string) {
    setDraft(text);
    composer?.focus();
  }

  function selectThread(machineId: string, threadId: string) {
    machines.select(machineId, threadId);
    stickToBottom = true;
  }

  /** A new thread, or a project added, opens on its machine, with the thread its server picks. */
  function sendAndOpen(machineId: string, message: ClientMessage) {
    sendTo(machineId, message);
    machines.select(machineId, null);
    stickToBottom = true;
  }

  const settingsDialog = $derived.by(() => {
    const open = dialog;
    if (open?.kind !== "settings") return null;
    const state = machines.get(open.machineId)?.server.state;
    const project = state?.projects.find((candidate) => candidate.id === open.projectId);
    return state && project ? { machineId: open.machineId, state, project } : null;
  });

  const toasts = $derived(machines.list.flatMap((entry) => entry.server.toasts.map((toast) => ({ entry, toast }))));
</script>

<svelte:window onfocus={onWindowFocus} onhashchange={onHashChange} onkeydown={onKeydown} />

{#if !machines.loaded}
  <div class="title-bar grid h-full place-items-center text-sm text-muted-foreground">Connecting to the Linker-Edit server…</div>
{:else if settings && settingsMachine && settingsApp}
  {@const shown = settingsMachine}
  <SettingsPage
    app={settingsApp}
    host={shown.host}
    machines={sidebarMachines}
    machineId={shown.id}
    onMachine={(id) => (settingsMachineId = id)}
    section={settings}
    onSection={showSection}
    onLibrary={() => openLibrary()}
    onSignOut={web.phase.kind === "app" ? () => web.signOut() : null}
    send={(message) => {
      // Its setup thread opens behind the settings page.
      if (message.type === "setup_collection") {
        sendAndOpen(shown.id, message);
        closeSettings();
      } else shown.server.send(message);
    }}
    onClose={closeSettings}
  />
{:else if library && app}
  <LibraryPage
    {app}
    tab={library}
    view={server.libraryView}
    connected={server.connected}
    onTab={showLibraryTab}
    send={(message) => server.send(message)}
    browseFolders={(path) => server.browseFolders(path)}
    sortFolder={(path) => server.sortLibraryFolder(path)}
    onClose={closeSettings}
  />
{:else}
  <div class="title-bar-row flex h-full">
    <Sidebar
      meta={active && app ? { machineId: active.id, app } : null}
      machines={sidebarMachines}
      projects={sidebarProjects}
      selected={active && app?.threadId ? { machineId: active.id, threadId: app.threadId } : null}
      send={sendTo}
      onSelect={selectThread}
      onNewThread={(machineId, projectId) => {
        sendAndOpen(machineId, { type: "new_thread", projectId });
        composer?.focus();
      }}
      onAddProject={() => (dialog = { kind: "add" })}
      onProjectSettings={(machineId, projectId) => (dialog = { kind: "settings", machineId, projectId })}
      onSettings={() => openSettings()}
      onSignOut={web.phase.kind === "app" ? () => web.signOut() : null}
    />

    <main class="flex min-w-0 flex-1 flex-col">
      {#if web.networkNotice || web.listProblem}
        <div class="title-bar flex shrink-0 items-center gap-2 border-b bg-accent/40 px-5 py-2 text-xs">
          {#if web.listProblem}
            <CircleAlert class="size-3.5 shrink-0 text-destructive" />
            <span class="min-w-0 flex-1 text-destructive">{web.listProblem}</span>
            <button type="button" class="shrink-0 rounded-md border px-2 py-1 hover:bg-accent" onclick={() => void web.list()}>Try again</button>
          {:else}
            <Network class="size-3.5 shrink-0 text-muted-foreground" />
            <span class="min-w-0 flex-1">
              Your browser may ask to let this page reach devices on your local network. Allow it: that's how it reaches your computers over
              Tailscale. If you don't, it uses the Linker Connect tunnel.
            </span>
            <button type="button" class="shrink-0 rounded-md border px-2 py-1 hover:bg-accent" onclick={() => web.dismissNetworkNotice()}>Got it</button>
          {/if}
        </div>
      {/if}
      {#if !app}
        <div class="title-bar grid flex-1 place-items-center px-5">
          {#if noComputers}
            <div class="flex max-w-sm flex-col items-center gap-3 text-center">
              <Monitor class="size-7 text-muted-foreground/70" />
              <h2 class="text-xl font-semibold tracking-tight">Link a computer</h2>
              <p class="text-sm text-muted-foreground">
                On the computer with Resolve, sign in to the same Linker account in Linker-Edit, and turn on Linker Connect or Tailscale
                remote access in its settings. Its projects then show here.
              </p>
              <button
                type="button"
                class="flex h-8 items-center gap-1.5 rounded-md border px-3 text-sm transition-colors hover:bg-accent"
                onclick={() => void web.list()}
              >
                <RefreshCw class="size-3.5" />
                Look again
              </button>
            </div>
          {:else if machines.list.length > 0 && machines.list.every((entry) => entry.status === "offline")}
            <p class="text-sm text-muted-foreground">
              {machines.list.length === 1 ? `${machines.list[0]!.name} is offline.` : "Your computers are offline."} Linker-Edit isn't running there, or
              it can't be reached from here.
            </p>
          {:else}
            <p class="text-sm text-muted-foreground">Connecting to the Linker-Edit server…</p>
          {/if}
        </div>
      {:else}
        <header class="title-bar flex h-13 shrink-0 items-center gap-2 border-b px-5">
          <h1 class="min-w-0 truncate text-sm font-medium">{chatTitle}</h1>
          {#if project}
            <span class="shrink-0 rounded-md border px-1.5 py-0.5 text-2xs text-muted-foreground" title={project.dir}>
              {project.name}
            </span>
          {/if}
          {#if active && machines.list.length > 1}
            <span class="flex shrink-0 items-center gap-1 text-2xs text-muted-foreground" title="On {active.name}">
              <Monitor class="size-3" />
              {active.name}
            </span>
          {/if}
          <div class="ml-auto flex shrink-0 items-center gap-1.5">
            <button
              type="button"
              class="flex h-7 items-center gap-1.5 rounded-md border px-2 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              title="Music, sound effects and graphics you reuse"
              onclick={() => openLibrary(project ? "project" : "universal")}
            >
              <Library class="size-3.5" />
              Library
            </button>
            {#each project?.setup ? [] : PANELS as toggle (toggle.name)}
              <button
                type="button"
                class="flex h-7 items-center gap-1.5 rounded-md border px-2 text-xs transition-colors {panel === toggle.name
                  ? 'bg-accent text-foreground'
                  : 'text-muted-foreground hover:bg-accent hover:text-foreground'}"
                aria-pressed={panel === toggle.name}
                onclick={() => togglePanel(toggle.name)}
              >
                <toggle.Icon class="size-3.5" />
                {toggle.label}
                {#if toggle.name === "preview" && rendering}
                  <span class="size-1.5 rounded-full bg-primary" title="Rendering"></span>
                {/if}
              </button>
            {/each}
          </div>
        </header>

        {#if !server.connected}
          <div class="flex items-center justify-center gap-2 border-b bg-warning/10 px-4 py-1.5 text-xs text-warning-foreground">
            <WifiOff class="size-3.5" />
            Lost connection to {machines.list.length > 1 && active ? active.name : "the Linker-Edit server"}. Reconnecting…
          </div>
        {/if}

        {#if !app.threadId}
          <div class="grid flex-1 place-items-center px-5">
            <div class="flex max-w-sm flex-col items-center gap-3 text-center">
              <FolderPlus class="size-7 text-muted-foreground/70" />
              <h2 class="text-xl font-semibold tracking-tight">Add a project</h2>
              <p class="text-sm text-muted-foreground">
                A project is one Resolve project with its own threads and edit history: on this computer, on a network database or in
                Blackmagic Cloud.
              </p>
              <button
                type="button"
                class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90"
                onclick={() => (dialog = { kind: "add" })}
              >
                Add project
              </button>
            </div>
          </div>
        {:else}
          <div class="min-h-0 flex-1 overflow-y-auto" bind:this={scroller} onscroll={onScroll}>
            <div class="mx-auto flex min-h-full w-full max-w-[46rem] flex-col gap-4 px-5 py-6">
              {#if app.items.length === 0 && project?.setup && collection}
                <ThreadStarter
                  title="Teach Linker-Edit how {collection.name} is edited"
                  intro="Point at videos and Resolve projects you've finished. The agent studies them, asks what it can't see, and proposes the rules for you to accept. It only looks at projects you've added here, opening them one by one in Resolve."
                  fields={[
                    { key: "finished", label: "Finished videos", hint: "A folder of videos as they were published. Leave it empty to learn from your Resolve projects only.", required: false },
                    { key: "raw", label: "Raw footage", hint: "What some of them were cut from. It shows what an editor leaves out.", required: false },
                    { key: "assets", label: "Brand assets", hint: "Logo, intro, music, LUTs.", required: false },
                  ]}
                  notesPlaceholder="Which finished Resolve projects to look at, which belong to which format, who watches them…"
                  button="Start"
                  disabled={startBlocked}
                  browseFolders={(path) => server.browseFolders(path)}
                  onStart={(folders, notes) =>
                    start(
                      [
                        `Set up the brief for ${collection.name}.`,
                        "",
                        "Finished Resolve projects: look at those of mine that fit, unless I name some below.",
                        `Finished videos: ${folders.finished || "none"}`,
                        `Raw footage: ${folders.raw || "none"}`,
                        `Brand assets: ${folders.assets || "none"}`,
                      ],
                      notes,
                    )}
                />
              {:else if app.items.length === 0 && canStartVideo && (startingVideo === app.threadId || project?.autonomous)}
                <NewVideoStarter
                  disabled={startBlocked}
                  browseFolders={(path) => server.browseFolders(path)}
                  onStart={(brief) => {
                    server.send({ type: "start_video", brief });
                    stickToBottom = true;
                  }}
                  onBack={project?.autonomous ? undefined : () => (startingVideo = null)}
                />
              {:else if app.items.length === 0}
                <div class="my-auto flex flex-col items-center gap-5 py-10 text-center">
                  <h2 class="text-2xl font-semibold tracking-tight">What are we editing?</h2>
                  {#if canStartVideo}
                    <button
                      type="button"
                      class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90"
                      onclick={() => (startingVideo = app?.threadId ?? null)}
                    >
                      New video
                    </button>
                  {/if}
                  <div class="flex max-w-xl flex-wrap justify-center gap-2">
                    {#each SUGGESTIONS as suggestion (suggestion)}
                      <button
                        type="button"
                        class="rounded-full border px-3 py-1.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                        onclick={() => useSuggestion(suggestion)}
                      >
                        {suggestion}
                      </button>
                    {/each}
                  </div>
                  <p class="text-xs text-muted-foreground/70">Working folder: {app.workspaceDir}</p>
                </div>
              {/if}

              {#if shownFrom > 0}
                <button
                  type="button"
                  class="self-center rounded-full border px-3 py-1 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  onclick={showEarlier}
                >
                  Show earlier
                </button>
              {/if}

              {#each shownRows as row (row.kind === "tools" ? row.id : row.item.id)}
                {#if row.kind === "tools"}
                  <div class="flex flex-col">
                    {#each row.items as tool (tool.id)}
                      <ToolRow item={tool} />
                    {/each}
                  </div>
                {:else if row.item.kind === "user"}
                  <div class="flex justify-end">
                    <div class="max-w-[80%] rounded-2xl bg-message p-3 text-sm leading-relaxed whitespace-pre-wrap">
                      {row.item.text}
                    </div>
                  </div>
                {:else if row.item.kind === "assistant"}
                  <div class="prose-chat px-1 select-text">{@html renderMarkdown(row.item.text)}</div>
                {:else if row.item.kind === "notice"}
                  {#if row.item.tone === "error"}
                    <div
                      class="flex items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/8 px-3 py-2 text-sm text-destructive"
                    >
                      <CircleAlert class="mt-0.5 size-4 shrink-0" />
                      <span class="min-w-0 whitespace-pre-wrap">{row.item.text}</span>
                    </div>
                  {:else}
                    <div class="px-1 text-xs text-muted-foreground">{row.item.text}</div>
                  {/if}
                {/if}
              {/each}

              {#if app.busy && app.approvals.length === 0}
                <WorkingIndicator />
              {/if}
            </div>
          </div>

          <div class="mx-auto flex w-full max-w-[46rem] shrink-0 flex-col gap-2 px-5 pb-4">
            {#each app.approvals as request (request.id)}
              <ApprovalCard {request} onAnswer={(allow) => server.send({ type: "approve", id: request.id, allow })} />
            {/each}
            {#if app.blocked}
              <div class="flex items-center gap-3 rounded-lg border border-warning/30 bg-warning/8 px-3 py-2 text-sm">
                <span class="min-w-0 flex-1 text-warning-foreground">{app.blocked.message}</span>
                {#if app.blocked.canOpen && app.projectId}
                  <button
                    type="button"
                    class="flex h-7 shrink-0 items-center gap-1.5 rounded-md border bg-card px-2.5 text-xs transition-colors hover:bg-accent"
                    onclick={() => app.projectId && server.send({ type: "open_in_resolve", projectId: app.projectId })}
                    title="Resolve saves the project it has open first"
                  >
                    <MonitorUp class="size-3.5" />
                    Open in Resolve
                  </button>
                {/if}
              </div>
            {/if}
            {#if app.reopen && !app.blocked}
              <div class="flex items-center gap-3 rounded-lg border px-3 py-2 text-sm">
                <span class="min-w-0 flex-1 text-muted-foreground">Resolve had "{app.reopen}" open before this. It was saved.</span>
                <button
                  type="button"
                  class="flex h-7 shrink-0 items-center gap-1.5 rounded-md border bg-card px-2.5 text-xs transition-colors hover:bg-accent"
                  onclick={() => server.send({ type: "reopen_previous" })}
                >
                  <MonitorUp class="size-3.5" />
                  Open it again
                </button>
              </div>
            {/if}
            <Composer
              bind:this={composer}
              bind:draft={() => draft, (value) => setDraft(value)}
              provider={app.provider}
              approvalMode={app.approvalMode}
              busy={app.busy}
              {canSend}
              context={app.context}
              {compactChip}
              onSend={send}
              onCompact={() => server.send({ type: "compact" })}
              onToggleCompactChip={() => (fullFor = compactChip === "compact" ? (app.context?.cacheExpiresAt ?? null) : null)}
              onStop={() => server.send({ type: "stop" })}
              onSelectProvider={selectProvider}
              onSetApprovalMode={(mode) => server.send({ type: "set_approval_mode", mode })}
            />
          </div>
        {/if}
      {/if}
    </main>

    {#if !app}
      <!-- Nothing to show beside the chat yet. -->
    {:else if panel === "history" && !project?.setup}
      <HistoryPanel
        entries={historyEntries}
        threadId={app.threadId}
        onClose={() => (panel = null)}
        onRestore={restore}
        onPreview={(id, commit) => openPreview({ id, commit })}
      />
    {:else if panel === "file" && !project?.setup}
      {#key app.projectId}
        <ProjectFilePanel
          file={projectFile}
          onSave={(text) => app?.projectId && server.send({ type: "save_project_file", projectId: app.projectId, text })}
          onClose={() => (panel = null)}
        />
      {/key}
    {:else if panel === "preview" && !project?.setup}
      <PreviewPanel
        projectId={app.projectId}
        preview={app.preview}
        resolve={app.resolve}
        busy={resolveBusy}
        target={currentTarget}
        onTarget={(target) => openPreview(target)}
        onRender={(timelineId) => server.send({ type: "render_preview", timelineId })}
        onApply={(timelineId) => app?.projectId && server.send({ type: "apply_timeline", projectId: app.projectId, timelineId })}
        onComment={commentAt}
        onStop={() => server.send({ type: "stop_preview" })}
        onClose={() => (panel = null)}
      />
    {/if}
  </div>

  {#if dialog?.kind === "add"}
    <AddProjectDialog
      machines={machines.list}
      onAdd={(machineId, source) => sendAndOpen(machineId, { type: "add_project", source })}
      onClose={() => (dialog = null)}
    />
  {:else if settingsDialog}
    {@const { machineId, state, project: edited } = settingsDialog}
    <ProjectSettingsDialog
      project={edited}
      collections={state.collections}
      relinkTo={state.resolve.state === "connected" && !state.projects.some((candidate) => candidate.open) ? state.resolve.project : null}
      onSave={(changes) => sendTo(machineId, { type: "update_project", projectId: edited.id, ...changes })}
      onMove={(collectionId) => sendTo(machineId, { type: "move_project", projectId: edited.id, collectionId })}
      onRelink={() => sendTo(machineId, { type: "relink_project", projectId: edited.id })}
      onRemove={() => sendTo(machineId, { type: "remove_project", projectId: edited.id })}
      onClose={() => (dialog = null)}
    />
  {/if}
{/if}

{#if toasts.length}
  <div class="fixed right-4 bottom-4 z-50 flex w-80 flex-col gap-2" role="status">
    {#each toasts as { entry, toast } (`${entry.id}:${toast.id}`)}
      <div
        class="flex items-start gap-2 rounded-lg border bg-card px-3 py-2 text-sm shadow-lg shadow-black/10 {toast.tone === 'error'
          ? 'text-destructive'
          : ''}"
      >
        <span class="min-w-0 flex-1">{toast.text}</span>
        <button type="button" class="text-muted-foreground hover:text-foreground" aria-label="Dismiss" onclick={() => entry.server.dismiss(toast.id)}>
          <X class="size-3.5" />
        </button>
      </div>
    {/each}
  </div>
{/if}
