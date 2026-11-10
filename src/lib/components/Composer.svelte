<script lang="ts">
  import { ArrowUp, ShieldCheck, ShieldOff, Square } from "@lucide/svelte";
  import type { ApprovalMode, ContextUsage, Provider } from "#shared/protocol.ts";
  import { formatTokens } from "#lib/context.ts";
  import { PROVIDER_LABEL } from "#lib/labels.ts";
  import ContextMeter from "./ContextMeter.svelte";
  import ProviderPicker from "./ProviderPicker.svelte";

  let {
    draft = $bindable(),
    provider,
    approvalMode,
    busy,
    canSend,
    context,
    compactChip,
    onSend,
    onStop,
    onSelectProvider,
    onSetApprovalMode,
    onCompact,
    onToggleCompactChip,
  }: {
    draft: string;
    provider: Provider;
    approvalMode: ApprovalMode;
    busy: boolean;
    canSend: boolean;
    context: ContextUsage | null;
    /** Shown when Claude's cache has expired on a big chat: "compact" summarizes before sending. */
    compactChip: "compact" | "full" | null;
    onSend: () => void;
    onStop: () => void;
    onSelectProvider: (provider: Provider) => void;
    onSetApprovalMode: (mode: ApprovalMode) => void;
    onCompact: () => void;
    onToggleCompactChip: () => void;
  } = $props();

  let textarea: HTMLTextAreaElement;
  export const focus = () => textarea.focus();

  const asking = $derived(approvalMode === "ask");

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      if (canSend) onSend();
    }
  }
</script>

<div class="relative rounded-2xl border border-composer-outline bg-card shadow-composer dark:shadow-none">
  <textarea
    bind:this={textarea}
    bind:value={draft}
    onkeydown={onKeydown}
    rows="2"
    placeholder="Ask {PROVIDER_LABEL[provider]} to edit…"
    class="field-sizing-content block max-h-64 min-h-16 w-full resize-none bg-transparent px-4 pt-3.5 pb-1 text-[0.9375rem] leading-relaxed outline-none placeholder:text-muted-foreground/70"
  ></textarea>
  <div class="flex items-center gap-1 px-2 pb-2">
    <ProviderPicker {provider} onSelect={onSelectProvider} />
    <span class="mx-0.5 h-4 w-px bg-border"></span>
    <button
      type="button"
      class="flex h-7 items-center gap-1.5 rounded-md px-2 text-sm text-muted-foreground transition-colors enabled:hover:bg-accent enabled:hover:text-foreground disabled:opacity-60"
      title={asking
        ? provider === "codex"
          ? "Resolve changes wait for your OK. Codex's shell commands and file edits run sandboxed in the project folder. Click to let everything run."
          : "Changes wait for your OK. Click to let everything run."
        : "Everything runs. Click to approve changes first."}
      onclick={() => onSetApprovalMode(asking ? "auto" : "ask")}
    >
      {#if asking}
        <ShieldCheck class="size-3.5" />
        Ask before changes
      {:else}
        <ShieldOff class="size-3.5" />
        Auto-approve
      {/if}
    </button>
    <div class="flex-1"></div>
    {#if context}
      <ContextMeter {context} canCompact={provider === "claude" && !busy} {onCompact} />
    {/if}
    {#if compactChip && context && !busy}
      <button
        type="button"
        class="flex h-7 items-center rounded-full border px-2.5 text-xs transition-colors hover:bg-accent {compactChip === 'compact'
          ? 'text-foreground'
          : 'text-muted-foreground'}"
        title={compactChip === "compact"
          ? `Claude's cache has expired. Sending summarizes the chat (${formatTokens(context.usedTokens)} tokens) first, so it isn't all reread. Click to send the full chat instead.`
          : "Sends the full chat. Click to summarize it first."}
        onclick={onToggleCompactChip}
      >
        {compactChip === "compact" ? "Compact" : "Full"}
      </button>
    {/if}
    {#if busy}
      <button
        type="button"
        class="flex size-8 items-center justify-center rounded-full bg-foreground text-background transition-opacity hover:opacity-85"
        aria-label="Stop"
        onclick={onStop}
      >
        <Square class="size-3 fill-current" />
      </button>
    {:else}
      <button
        type="button"
        class="flex size-8 items-center justify-center rounded-full bg-primary text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-30"
        aria-label="Send"
        disabled={!canSend}
        onclick={onSend}
      >
        <ArrowUp class="size-4" strokeWidth={2.5} />
      </button>
    {/if}
  </div>
</div>
