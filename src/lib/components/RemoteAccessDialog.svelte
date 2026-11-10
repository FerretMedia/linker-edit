<script lang="ts">
  import { onMount } from "svelte";
  import { Check, Copy } from "@lucide/svelte";
  import type { RemoteAccess } from "#shared/protocol.ts";
  import Dialog from "./Dialog.svelte";

  let {
    remote,
    onRefresh,
    onSetEnabled,
    onClose,
  }: {
    remote: RemoteAccess;
    onRefresh: () => void;
    onSetEnabled: (enabled: boolean) => void;
    onClose: () => void;
  } = $props();

  // Tailscale may have changed since the app last looked, such as after logging in.
  onMount(() => onRefresh());

  let copied = $state(false);
  const url = $derived(remote.state === "off" || remote.state === "on" || remote.state === "switching" ? remote.url : null);
  // Turning it off from a remote device cuts that device off.
  const viaTailscale = !["localhost", "127.0.0.1", "[::1]"].includes(location.hostname);

  /** Text with its links made clickable, as Tailscale's hints often carry one. */
  function parts(text: string): { text: string; href?: string }[] {
    return text.split(/(https:\/\/\S+[^\s.,)])/).map((part) => (part.startsWith("https://") ? { text: part, href: part } : { text: part }));
  }

  async function copy() {
    if (!url) return;
    await navigator.clipboard.writeText(url);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  function turnOff() {
    if (viaTailscale && !confirm("You're connected through Tailscale. Turning remote access off disconnects this device.")) return;
    onSetEnabled(false);
  }
</script>

{#snippet linked(text: string)}
  {#each parts(text) as part, index (index)}
    {#if part.href}
      <a class="break-all text-primary underline underline-offset-2" href={part.href} target="_blank" rel="noreferrer">{part.text}</a>
    {:else}
      {part.text}
    {/if}
  {/each}
{/snippet}

<Dialog title="Remote access" {onClose}>
  <p class="text-sm text-muted-foreground">
    Let your other computers use this one over <a class="text-primary underline underline-offset-2" href="https://tailscale.com" target="_blank" rel="noreferrer">Tailscale</a>.
    Resolve and the agents keep running here; only devices logged in to your Tailscale account get in.
  </p>

  {#if remote.state === "checking"}
    <p class="text-sm text-muted-foreground">Checking Tailscale…</p>
  {:else if remote.state === "unavailable"}
    <p class="text-sm">{@render linked(remote.hint)}</p>
    <button type="button" class="h-8 self-start rounded-md border px-2.5 text-sm transition-colors hover:bg-accent" onclick={onRefresh}>
      Check again
    </button>
  {:else}
    {#if url}
      <div class="flex flex-col gap-1 text-xs">
        <span class="text-muted-foreground">
          {remote.state === "on" ? "On your other computer, pick this one in Linker-Edit's computer menu, or open" : "Address once it's on"}
        </span>
        <div class="flex items-center gap-2">
          <p class="min-w-0 flex-1 font-mono text-xs break-all select-all {remote.state === 'on' ? '' : 'text-muted-foreground'}">{url}</p>
          {#if remote.state === "on"}
            <button
              type="button"
              class="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
              aria-label="Copy address"
              title="Copy address"
              onclick={copy}
            >
              {#if copied}<Check class="size-3.5" />{:else}<Copy class="size-3.5" />{/if}
            </button>
          {/if}
        </div>
      </div>
    {/if}

    {#if remote.state === "on"}
      <p class="text-xs text-muted-foreground">
        For devices logged in as <span class="text-foreground">{remote.owner}</span>. It stays on after Linker-Edit restarts, until you turn it off.
      </p>
    {:else if remote.state === "off" && remote.problem}
      <p class="text-sm text-destructive">{@render linked(remote.problem)}</p>
    {/if}

    <div class="flex items-center gap-2 border-t pt-4">
      <span class="flex items-center gap-2 text-sm">
        <span class="size-2 rounded-full {remote.state === 'on' ? 'bg-success' : remote.state === 'switching' ? 'bg-warning' : 'bg-muted-foreground'}"></span>
        {remote.state === "on" ? "On" : remote.state === "switching" ? "Switching…" : "Off"}
      </span>
      {#if remote.state === "on"}
        <button type="button" class="ml-auto h-8 rounded-md border px-3 text-sm transition-colors hover:bg-accent" onclick={turnOff}>Turn off</button>
      {:else}
        <button
          type="button"
          class="ml-auto h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
          disabled={remote.state === "switching"}
          onclick={() => onSetEnabled(true)}
        >
          Turn on
        </button>
      {/if}
    </div>
  {/if}
</Dialog>
