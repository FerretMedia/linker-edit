<script lang="ts">
  import { ArrowUp, ShieldCheck, ShieldOff, Square } from "@lucide/svelte";
  import type { ApprovalMode, Provider } from "#shared/protocol.ts";
  import { PROVIDER_LABEL } from "#lib/labels.ts";
  import ProviderPicker from "./ProviderPicker.svelte";

  let {
    draft = $bindable(),
    provider,
    approvalMode,
    busy,
    canSend,
    onSend,
    onStop,
    onSelectProvider,
    onSetApprovalMode,
  }: {
    draft: string;
    provider: Provider;
    approvalMode: ApprovalMode;
    busy: boolean;
    canSend: boolean;
    onSend: () => void;
    onStop: () => void;
    onSelectProvider: (provider: Provider) => void;
    onSetApprovalMode: (mode: ApprovalMode) => void;
  } = $props();

  let textarea: HTMLTextAreaElement;
  export const focus = () => textarea.focus();

  const askLocked = $derived(provider === "codex");
  const asking = $derived(approvalMode === "ask" && !askLocked);

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
      disabled={askLocked}
      title={askLocked
        ? "Codex runs without per-call approvals"
        : asking
          ? "Changes wait for your OK. Click to let everything run."
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
