<script lang="ts">
  import { CircleAlert, History, MonitorPlay, WifiOff } from "@lucide/svelte";
  import type { ChatItem, HistoryEntry, Provider } from "#shared/protocol.ts";
  import ApprovalCard from "#lib/components/ApprovalCard.svelte";
  import Composer from "#lib/components/Composer.svelte";
  import HistoryPanel from "#lib/components/HistoryPanel.svelte";
  import PreviewPanel from "#lib/components/PreviewPanel.svelte";
  import Sidebar from "#lib/components/Sidebar.svelte";
  import ToolRow from "#lib/components/ToolRow.svelte";
  import WorkingIndicator from "#lib/components/WorkingIndicator.svelte";
  import { renderMarkdown } from "#lib/markdown.ts";
  import type { PreviewTarget } from "#lib/player.ts";
  import { server } from "#lib/connection.svelte.ts";

  const SUGGESTIONS = [
    "Describe the open project and its timelines",
    "What's in the media pool?",
    "Duplicate the current timeline and remove the silences",
  ];

  server.connect();

  let draft = $state("");
  let composer: ReturnType<typeof Composer> | undefined = $state();
  let scroller: HTMLElement | undefined = $state();
  let stickToBottom = true;
  // One side panel at a time, opened from the header like T3 Code's diff panel.
  let panel = $state<"preview" | "history" | null>(null);
  const historyOpen = $derived(panel === "history");

  const app = $derived(server.state);
  const rendering = $derived(app?.preview.state === "rendering");
  const canSend = $derived(server.connected && !!app && !app.busy && !rendering && draft.trim().length > 0);
  const chatTitle = $derived.by(() => {
    const first = app?.items.find((item) => item.kind === "user");
    return first ? first.text.split("\n")[0]!.slice(0, 80) : "New chat";
  });

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
    // Asks again after a reconnect; while open, the server pushes every new save.
    if (historyOpen && server.connected) server.send({ type: "load_history" });
  });

  function restore(entry: HistoryEntry) {
    draft = `Restore the timelines to how they were in save ${entry.commit} ("${entry.subject}").`;
    composer?.focus();
  }

  function togglePanel(name: "preview" | "history") {
    panel = panel === name ? null : name;
  }

  // The current timeline until a version is picked from the history.
  let previewTarget = $state<PreviewTarget>({ id: null, commit: null });

  function openPreview(target: PreviewTarget) {
    previewTarget = target;
    panel = "preview";
  }

  function onScroll() {
    if (!scroller) return;
    stickToBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 48;
  }

  function send() {
    if (!canSend) return;
    server.send({ type: "send", text: draft.trim() });
    draft = "";
    stickToBottom = true;
  }

  function selectProvider(provider: Provider) {
    if (app?.items.length && !confirm("Switching agent starts a new chat. Continue?")) return;
    server.send({ type: "set_provider", provider });
  }

  function useSuggestion(text: string) {
    draft = text;
    composer?.focus();
  }
</script>

{#if !app}
  <div class="grid h-full place-items-center text-sm text-muted-foreground">
    {server.connected ? "Loading…" : "Connecting to the Linker-Edit server…"}
  </div>
{:else}
  <div class="flex h-full">
    <Sidebar
      resolve={app.resolve}
      version={app.version}
      update={app.update}
      {chatTitle}
      busy={app.busy}
      onNewChat={() => server.send({ type: "new_chat" })}
      onRefreshResolve={() => server.send({ type: "refresh_resolve" })}
    />

    <main class="flex min-w-0 flex-1 flex-col">
      <header class="flex h-13 shrink-0 items-center gap-2 border-b px-5">
        <h1 class="min-w-0 truncate text-sm font-medium">{chatTitle}</h1>
        {#if app.resolve.state === "connected" && app.resolve.project}
          <span class="shrink-0 rounded-md border px-1.5 py-0.5 text-2xs text-muted-foreground">
            {app.resolve.project}
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
        <Composer
          bind:this={composer}
          bind:draft
          provider={app.provider}
          approvalMode={app.approvalMode}
          busy={app.busy}
          {canSend}
          onSend={send}
          onStop={() => server.send({ type: "stop" })}
          onSelectProvider={selectProvider}
          onSetApprovalMode={(mode) => server.send({ type: "set_approval_mode", mode })}
        />
      </div>
    </main>

    {#if panel === "history"}
      <HistoryPanel entries={server.history} onClose={() => (panel = null)} onRestore={restore} onPreview={(id, commit) => openPreview({ id, commit })} />
    {:else if panel === "preview"}
      <PreviewPanel
        preview={app.preview}
        resolve={app.resolve}
        busy={app.busy}
        target={previewTarget}
        onTarget={(target) => (previewTarget = target)}
        onRender={(timelineId) => server.send({ type: "render_preview", timelineId })}
        onStop={() => server.send({ type: "stop_preview" })}
        onClose={() => (panel = null)}
      />
    {/if}
  </div>
{/if}
