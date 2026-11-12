<script lang="ts">
  import { CircleAlert, FolderPlus, History, MonitorPlay, MonitorUp, WifiOff, X } from "@lucide/svelte";
  import type { ChatItem, HistoryEntry, Provider } from "#shared/protocol.ts";
  import AddProjectDialog from "#lib/components/AddProjectDialog.svelte";
  import ApprovalCard from "#lib/components/ApprovalCard.svelte";
  import Composer from "#lib/components/Composer.svelte";
  import HistoryPanel from "#lib/components/HistoryPanel.svelte";
  import PreviewPanel from "#lib/components/PreviewPanel.svelte";
  import ProjectSettingsDialog from "#lib/components/ProjectSettingsDialog.svelte";
  import RemoteAccessDialog from "#lib/components/RemoteAccessDialog.svelte";
  import Sidebar from "#lib/components/Sidebar.svelte";
  import ToolRow from "#lib/components/ToolRow.svelte";
  import WorkingIndicator from "#lib/components/WorkingIndicator.svelte";
  import { compactSuggested } from "#lib/context.ts";
  import { renderMarkdown } from "#lib/markdown.ts";
  import type { PreviewTarget } from "#lib/player.ts";
  import { server } from "#lib/connection.svelte.ts";

  const SUGGESTIONS = [
    "Describe the open project and its timelines",
    "What's in the media pool?",
    "Duplicate the current timeline and remove the silences",
  ];

  server.connect();

  // One draft per thread, like T3 Code, so switching threads never sends a draft to the wrong one.
  let drafts = $state<Record<string, string>>({});
  const draftKey = $derived(server.state?.threadId ?? "");
  const draft = $derived(drafts[draftKey] ?? "");
  const setDraft = (value: string, key = draftKey) => (drafts[key] = value);

  server.onRefused = (threadId, text) => {
    if (!(drafts[threadId] ?? "").trim()) setDraft(text, threadId);
  };

  let dialog = $state<{ kind: "add" } | { kind: "settings"; projectId: string } | { kind: "remote" } | null>(null);
  let composer: ReturnType<typeof Composer> | undefined = $state();
  let scroller: HTMLElement | undefined = $state();
  let stickToBottom = true;
  // One side panel at a time, opened from the header like T3 Code's diff panel.
  let panel = $state<"preview" | "history" | null>(null);
  const historyOpen = $derived(panel === "history");

  const app = $derived(server.state);
  const project = $derived(app?.projects.find((candidate) => candidate.id === app.projectId) ?? null);
  const thread = $derived(project?.threads.find((candidate) => candidate.id === app?.threadId) ?? null);
  const rendering = $derived(app?.preview.state === "rendering");
  const canSend = $derived(
    server.connected && !!app?.threadId && !app.busy && !app.blocked && !rendering && draft.trim().length > 0,
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

  const historyEntries = $derived(server.history && server.history.projectId === app?.projectId ? server.history.entries : null);

  function restore(entry: HistoryEntry) {
    setDraft(`Restore the timelines to how they were in save ${entry.commit} ("${entry.subject}").`);
    composer?.focus();
  }

  function togglePanel(name: "preview" | "history") {
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
    if (server.connected) server.send({ type: "refresh_resolve" });
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

  function selectThread(threadId: string) {
    server.send({ type: "select_thread", threadId });
    stickToBottom = true;
  }

  function loadLibrary() {
    server.library = null;
    server.send({ type: "load_library" });
  }

  const settingsProject = $derived.by(() => {
    const open = dialog;
    return open?.kind === "settings" ? (app?.projects.find((candidate) => candidate.id === open.projectId) ?? null) : null;
  });
</script>

<svelte:window onfocus={onWindowFocus} />

{#if !app}
  <div class="title-bar grid h-full place-items-center text-sm text-muted-foreground">
    {server.connected ? "Loading…" : "Connecting to the Linker-Edit server…"}
  </div>
{:else}
  <div class="title-bar-row flex h-full">
    <Sidebar
      resolve={app.resolve}
      version={app.version}
      update={app.update}
      appUpdate={app.appUpdate}
      transcription={app.transcription}
      remote={app.remote}
      computer={app.computer}
      linker={app.linker}
      projects={app.projects}
      threadId={app.threadId}
      onSelect={selectThread}
      onNewThread={(projectId) => {
        server.send({ type: "new_thread", projectId });
        composer?.focus();
      }}
      onRenameThread={(threadId, title) => server.send({ type: "rename_thread", threadId, title })}
      onArchiveThread={(threadId, archived) => server.send({ type: "archive_thread", threadId, archived })}
      onDeleteThread={(threadId) => server.send({ type: "delete_thread", threadId })}
      onAddProject={() => (dialog = { kind: "add" })}
      onAddOpenProject={() => server.send({ type: "add_project", source: { kind: "open" } })}
      onProjectSettings={(projectId) => (dialog = { kind: "settings", projectId })}
      onOpenInResolve={(projectId) => server.send({ type: "open_in_resolve", projectId })}
      onRefreshResolve={() => server.send({ type: "refresh_resolve" })}
      onRemoteAccess={() => (dialog = { kind: "remote" })}
      onLinkerCancel={() => server.send({ type: "linker_cancel_sign_in" })}
      onLinkerSignOut={() => server.send({ type: "linker_sign_out" })}
      onCheckAppUpdate={() => server.send({ type: "check_app_update" })}
      onInstallAppUpdate={() => server.send({ type: "install_app_update" })}
      onCancelAppUpdate={() => server.send({ type: "cancel_app_update" })}
    />

    <main class="flex min-w-0 flex-1 flex-col">
      <header class="title-bar flex h-13 shrink-0 items-center gap-2 border-b px-5">
        <h1 class="min-w-0 truncate text-sm font-medium">{chatTitle}</h1>
        {#if project}
          <span class="shrink-0 rounded-md border px-1.5 py-0.5 text-2xs text-muted-foreground" title={project.dir}>
            {project.name}
          </span>
        {/if}
        <div class="ml-auto flex shrink-0 items-center gap-1.5">
          {#each [{ name: "preview", label: "Preview", Icon: MonitorPlay }, { name: "history", label: "History", Icon: History }] as const as toggle (toggle.name)}
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
          Lost connection to the Linker-Edit server. Reconnecting…
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
            {#if app.items.length === 0}
              <div class="my-auto flex flex-col items-center gap-5 py-10 text-center">
                <h2 class="text-2xl font-semibold tracking-tight">What are we editing?</h2>
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

            {#each rows as row (row.kind === "tools" ? row.id : row.item.id)}
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
    </main>

    {#if panel === "history"}
      <HistoryPanel
        entries={historyEntries}
        threadId={app.threadId}
        onClose={() => (panel = null)}
        onRestore={restore}
        onPreview={(id, commit) => openPreview({ id, commit })}
      />
    {:else if panel === "preview"}
      <PreviewPanel
        projectId={app.projectId}
        preview={app.preview}
        resolve={app.resolve}
        busy={resolveBusy}
        target={currentTarget}
        onTarget={(target) => openPreview(target)}
        onRender={(timelineId) => server.send({ type: "render_preview", timelineId })}
        onApply={(timelineId) => app?.projectId && server.send({ type: "apply_timeline", projectId: app.projectId, timelineId })}
        onStop={() => server.send({ type: "stop_preview" })}
        onClose={() => (panel = null)}
      />
    {/if}
  </div>

  {#if dialog?.kind === "add"}
    <AddProjectDialog
      resolve={app.resolve}
      library={server.library}
      onLoadLibrary={loadLibrary}
      browseFolders={(path) => server.browseFolders(path)}
      onAdd={(source) => server.send({ type: "add_project", source })}
      onClose={() => (dialog = null)}
    />
  {:else if dialog?.kind === "remote"}
    <RemoteAccessDialog
      remote={app.remote}
      onRefresh={() => server.send({ type: "refresh_remote_access" })}
      onSetEnabled={(enabled) => server.send({ type: "set_remote_access", enabled })}
      onClose={() => (dialog = null)}
    />
  {:else if settingsProject}
    <ProjectSettingsDialog
      project={settingsProject}
      relinkTo={app.resolve.state === "connected" && !app.projects.some((candidate) => candidate.open) ? app.resolve.project : null}
      browseFolders={(path) => server.browseFolders(path)}
      onSave={(changes) => server.send({ type: "update_project", projectId: settingsProject.id, ...changes })}
      onRelink={() => server.send({ type: "relink_project", projectId: settingsProject.id })}
      onRemove={() => server.send({ type: "remove_project", projectId: settingsProject.id })}
      onClose={() => (dialog = null)}
    />
  {/if}

  {#if server.toasts.length}
    <div class="fixed right-4 bottom-4 z-50 flex w-80 flex-col gap-2" role="status">
      {#each server.toasts as toast (toast.id)}
        <div
          class="flex items-start gap-2 rounded-lg border bg-card px-3 py-2 text-sm shadow-lg shadow-black/10 {toast.tone === 'error'
            ? 'text-destructive'
            : ''}"
        >
          <span class="min-w-0 flex-1">{toast.text}</span>
          <button type="button" class="text-muted-foreground hover:text-foreground" aria-label="Dismiss" onclick={() => server.dismiss(toast.id)}>
            <X class="size-3.5" />
          </button>
        </div>
      {/each}
    </div>
  {/if}
{/if}
