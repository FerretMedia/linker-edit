<script lang="ts">
  import type { ChatItem } from "#shared/protocol.ts";
  import { formatJson, isResolveTool, toolLabel, toolSummary } from "#lib/labels.ts";
  import ToolIcon, { type ToolIconKind } from "./ToolIcon.svelte";

  let { item }: { item: Extract<ChatItem, { kind: "tool" }> } = $props();
  let expanded = $state(false);

  const icon = $derived.by((): ToolIconKind => {
    const name = item.name;
    if (isResolveTool(name)) return "resolve";
    if (name === "shell" || name === "Bash") return "terminal";
    if (name === "Read") return "read";
    if (name === "Edit" || name === "Write" || name === "edit files") return "edit";
    if (name === "Grep" || name === "Glob") return "search";
    if (name === "WebSearch" || name === "WebFetch" || name === "web search") return "web";
    if (name === "TodoWrite") return "todo";
    if (name === "ToolSearch") return "schema";
    return "tool";
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
      <ToolIcon kind={icon} class="size-4" />
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
      <ToolIcon kind="x" class="size-3 shrink-0 text-destructive" />
    {/if}
    <ToolIcon
      kind="chevron"
      class="size-3 shrink-0 text-muted-foreground opacity-70 transition-transform duration-200 {expanded ? 'rotate-90' : ''}"
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
