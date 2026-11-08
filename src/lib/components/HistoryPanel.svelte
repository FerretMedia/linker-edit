<script lang="ts">
  import { ChevronRight, History, RotateCcw, UserRound, X } from "@lucide/svelte";
  import type { HistoryEntry } from "#shared/protocol.ts";
  import ClaudeIcon from "#lib/components/ClaudeIcon.svelte";
  import ClipStrip from "#lib/components/ClipStrip.svelte";
  import OpenAIIcon from "#lib/components/OpenAIIcon.svelte";
  import { CHANGE_STYLE, dayLabel, splitChange } from "#lib/labels.ts";

  let {
    entries,
    onClose,
    onRestore,
    onPreview,
  }: {
    entries: HistoryEntry[] | null;
    onClose: () => void;
    onRestore: (entry: HistoryEntry) => void;
    onPreview: (timelineId: string, commit: string) => void;
  } = $props();

  let expanded = $state(new Set<string>());

  const timeFormat = new Intl.DateTimeFormat(undefined, { hour: "2-digit", minute: "2-digit" });

  // One list for the whole rail, so it runs unbroken through the day headings.
  type Row = { kind: "day"; label: string } | { kind: "save"; entry: HistoryEntry };
  const rows = $derived.by(() => {
    const result: Row[] = [];
    const now = new Date();
    let day = "";
    for (const entry of entries ?? []) {
      const label = dayLabel(new Date(entry.date), now);
      if (label !== day) result.push({ kind: "day", label });
      day = label;
      result.push({ kind: "save", entry });
    }
    return result;
  });

  /** Each strip with its line from the summary, and the lines no strip can show (markers, removed timelines). */
  function layout(entry: HistoryEntry) {
    const lines = entry.changes.map(splitChange);
    const detailFor = (name: string) => lines.find((line) => line.timeline === name)?.detail ?? null;
    const shown = new Set(entry.timelines.map((strip) => strip.name));
    return {
      strips: entry.timelines.map((strip) => ({ strip, detail: detailFor(strip.name) })),
      rest: lines.filter((line) => !line.timeline || !shown.has(line.timeline)),
    };
  }

  function toggle(commit: string) {
    const next = new Set(expanded);
    if (!next.delete(commit)) next.add(commit);
    expanded = next;
  }
</script>

<aside class="flex w-[24rem] shrink-0 flex-col border-l bg-sidebar">
  <header class="flex h-13 shrink-0 items-center gap-2 border-b px-4">
    <History class="size-4 text-muted-foreground" />
    <h2 class="text-sm font-medium">Edit history</h2>
    {#if entries?.length}
      <span class="rounded-md border px-1.5 py-0.5 text-2xs text-muted-foreground">{entries.length}</span>
    {/if}
    <button
      type="button"
      class="ml-auto flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      onclick={onClose}
      title="Close history"
    >
      <X class="size-4" />
    </button>
  </header>

  {#if entries?.length}
    <div class="flex flex-wrap items-center gap-x-3 gap-y-1 border-b px-4 py-2 text-2xs text-muted-foreground">
      {#each Object.entries(CHANGE_STYLE) as [change, style] (change)}
        <span class="flex items-center gap-1">
          <span class="size-2 rounded-full {style.dot}"></span>
          {change}
        </span>
      {/each}
    </div>
  {/if}

  <div class="min-h-0 flex-1 overflow-y-auto px-3 py-3">
    {#if entries === null}
      <p class="px-1 py-6 text-center text-xs text-muted-foreground">Loading history…</p>
    {:else if entries.length === 0}
      <div class="flex flex-col items-center gap-2 px-4 py-12 text-center">
        <History class="size-6 text-muted-foreground/60" />
        <p class="text-sm font-medium">No saves yet</p>
        <p class="text-xs text-muted-foreground">
          Linker-Edit saves your Resolve project before each turn and after every turn that changes it.
        </p>
      </div>
    {/if}

    <ol>
      {#each rows as row, index (row.kind === "day" ? `day:${row.label}` : row.entry.commit)}
        {@const last = index === rows.length - 1}
        {#if row.kind === "day"}
          <li class="relative flex h-7 items-center gap-3">
            {#if index > 0}<span class="absolute top-0 bottom-0 left-[0.8125rem] w-px bg-border"></span>{/if}
            <span class="relative w-[1.625rem] shrink-0"></span>
            <h3 class="text-2xs font-medium tracking-wide text-muted-foreground uppercase">{row.label}</h3>
          </li>
        {:else}
          {@const entry = row.entry}
          {@const open = expanded.has(entry.commit)}
          {@const { strips, rest } = layout(entry)}
          <li class="relative flex gap-3 pb-5">
            {#if !last}
              <span class="absolute top-0 bottom-0 left-[0.8125rem] w-px bg-border"></span>
            {/if}
            <span
              class="relative flex size-[1.625rem] shrink-0 items-center justify-center rounded-full border bg-card shadow-[0_0_0_3px_var(--sidebar)]"
            >
              {#if entry.agent === "claude"}
                <ClaudeIcon class="size-3.5" />
              {:else if entry.agent === "codex"}
                <OpenAIIcon class="size-3.5" />
              {:else}
                <UserRound class="size-3.5 text-muted-foreground" />
              {/if}
            </span>

            <div class="min-w-0 flex-1 pt-0.5">
              <p class="line-clamp-2 text-sm leading-snug font-medium" title={entry.subject}>{entry.subject}</p>
              <p class="mt-0.5 flex items-center gap-1.5 text-2xs text-muted-foreground">
                <span>{entry.author}</span>
                <span aria-hidden="true">·</span>
                <time datetime={entry.date}>{timeFormat.format(new Date(entry.date))}</time>
                <span aria-hidden="true">·</span>
                <span class="font-mono">{entry.commit}</span>
              </p>

              {#if strips.length || rest.length}
                <div class="mt-2 flex flex-col gap-3 rounded-lg border bg-card p-2.5">
                  {#each strips as { strip, detail } (strip.name)}
                    <ClipStrip {strip} {detail} onPreview={(id) => onPreview(id, entry.commit)} />
                  {/each}
                  {#each rest as line, lineIndex (lineIndex)}
                    <p class="text-xs leading-snug">
                      {#if line.timeline}<span class="font-medium">{line.timeline}</span>{/if}
                      <span class="text-muted-foreground">{line.detail}</span>
                    </p>
                  {/each}
                </div>
              {/if}

              <div class="mt-1.5 flex items-center gap-1">
                {#if entry.prompt || entry.actions.length}
                  <button
                    type="button"
                    class="flex h-6 items-center gap-1 rounded-md px-1.5 text-2xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                    aria-expanded={open}
                    onclick={() => toggle(entry.commit)}
                  >
                    <ChevronRight class="size-3 transition-transform duration-200 {open ? 'rotate-90' : ''}" />
                    {entry.actions.length} Resolve {entry.actions.length === 1 ? "action" : "actions"}
                  </button>
                {/if}
                <button
                  type="button"
                  class="flex h-6 items-center gap-1 rounded-md px-1.5 text-2xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
                  onclick={() => onRestore(entry)}
                  title="Ask the agent to bring the timelines back to this save"
                >
                  <RotateCcw class="size-3" />
                  Restore
                </button>
              </div>

              {#if open}
                <div class="mt-1 flex flex-col gap-2 ps-1.5 select-text">
                  {#if entry.prompt}
                    <p class="border-l-2 ps-2.5 text-xs leading-relaxed whitespace-pre-wrap text-muted-foreground">{entry.prompt}</p>
                  {/if}
                  {#each entry.actions as action, actionIndex (actionIndex)}
                    <code class="font-mono text-2xs leading-relaxed break-all text-muted-foreground">{action}</code>
                  {/each}
                </div>
              {/if}
            </div>
          </li>
        {/if}
      {/each}
    </ol>
  </div>
</aside>
