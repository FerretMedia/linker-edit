<script lang="ts">
  import { Check, ChevronDown } from "@lucide/svelte";
  import type { Provider } from "#shared/protocol.ts";
  import { PROVIDER_LABEL } from "#lib/labels.ts";
  import ClaudeIcon from "./ClaudeIcon.svelte";
  import OpenAIIcon from "./OpenAIIcon.svelte";

  let { provider, onSelect }: { provider: Provider; onSelect: (provider: Provider) => void } = $props();
  let open = $state(false);
  let root: HTMLDivElement;

  function closeOnOutside(event: PointerEvent) {
    if (open && !root.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={closeOnOutside} onkeydown={(event) => event.key === "Escape" && (open = false)} />

{#snippet providerIcon(value: Provider)}
  {#if value === "claude"}
    <ClaudeIcon class="size-3.5" />
  {:else}
    <OpenAIIcon class="size-3.5" />
  {/if}
{/snippet}

<div class="relative" bind:this={root}>
  <button
    type="button"
    class="flex h-7 items-center gap-1.5 rounded-md px-2 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
    aria-haspopup="menu"
    aria-expanded={open}
    onclick={() => (open = !open)}
  >
    {@render providerIcon(provider)}
    {PROVIDER_LABEL[provider]}
    <ChevronDown class="size-3 opacity-70" />
  </button>
  {#if open}
    <div
      class="absolute bottom-full left-0 z-50 mb-1.5 w-44 rounded-lg border bg-card p-1 shadow-lg shadow-black/10"
      role="menu"
    >
      {#each ["claude", "codex"] as const as value (value)}
        <button
          type="button"
          role="menuitemradio"
          aria-checked={provider === value}
          class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-sm transition-colors hover:bg-accent"
          onclick={() => {
            open = false;
            if (value !== provider) onSelect(value);
          }}
        >
          {@render providerIcon(value)}
          <span class="flex-1 text-left">{PROVIDER_LABEL[value]}</span>
          {#if provider === value}
            <Check class="size-3.5 text-muted-foreground" />
          {/if}
        </button>
      {/each}
    </div>
  {/if}
</div>
