<script lang="ts">
  import {
    BookOpen,
    ChevronRight,
    Clapperboard,
    FilePen,
    FileText,
    Globe,
    ListTodo,
    Search,
    SquareTerminal,
    Wrench,
    X,
  } from "@lucide/svelte";
  import type { ChatItem } from "#shared/protocol.ts";
  import { formatJson, isResolveTool, toolLabel, toolSummary } from "#lib/labels.ts";

  let { item }: { item: Extract<ChatItem, { kind: "tool" }> } = $props();
  let expanded = $state(false);

  const Icon = $derived.by(() => {
    const name = item.name;
    if (isResolveTool(name)) return Clapperboard;
    if (name === "shell" || name === "Bash") return SquareTerminal;
    if (name === "Read") return FileText;
    if (name === "Edit" || name === "Write" || name === "edit files") return FilePen;
    if (name === "Grep" || name === "Glob") return Search;
    if (name === "WebSearch" || name === "WebFetch" || name === "web search") return Globe;
    if (name === "TodoWrite") return ListTodo;
    if (name === "ToolSearch") return BookOpen;
    return Wrench;
  });
  const label = $derived(toolLabel(item.name, item.input));
  const summary = $derived(toolSummary(item.name, item.input));
</script>

<div class="w-full min-w-0 rounded-md px-0.5 py-0.5">
  <button
    type="button"
    class="flex w-full min-w-0 items-center gap-1.5 rounded-md text-left text-sm leading-relaxed focus-visible:ring-2 focus-visible:ring-primary/70 focus-visible:outline-none"
    aria-expanded={expanded}
    onclick={() => (expanded = !expanded)}
  >
    <span
      class="flex size-6 shrink-0 items-center justify-center {item.status === 'error'
        ? 'text-destructive'
        : 'text-muted-foreground'}"
    >
      <Icon class="size-4" strokeWidth={2} />
    </span>
    <span class="flex min-w-0 flex-1 items-baseline gap-1.5 truncate">
      <span class="shrink-0 {item.status === 'error' ? 'font-medium text-destructive' : 'text-foreground/80'}">
        {label}
      </span>
      {#if summary}
        <span class="truncate text-muted-foreground">{summary}</span>
      {/if}
    </span>
    {#if item.status === "running"}
      <span class="shrink-0 text-xs text-muted-foreground">Running</span>
    {:else if item.status === "error"}
      <X class="size-3 shrink-0 text-destructive" />
    {/if}
    <ChevronRight
      class="size-3 shrink-0 text-muted-foreground opacity-70 transition-transform duration-200 {expanded
        ? 'rotate-90'
        : ''}"
    />
  </button>
  {#if expanded}
    <div class="ms-7 flex max-h-96 flex-col gap-2 overflow-auto px-0.5 py-1 select-text">
      <pre class="font-mono text-2xs leading-relaxed whitespace-pre-wrap break-words text-muted-foreground">{formatJson(
          item.input,
        )}</pre>
      {#if item.output}
        <pre
          class="border-l-2 ps-2.5 font-mono text-2xs leading-relaxed whitespace-pre-wrap break-words text-muted-foreground">{item.output}</pre>
      {/if}
    </div>
  {/if}
</div>
