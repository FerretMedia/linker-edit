<script lang="ts">
  import { onMount } from "svelte";
  import { Check, ChevronsUpDown, Globe, Monitor } from "@lucide/svelte";
  import type { Computers, RemoteAccess } from "#shared/protocol.ts";

  interface DesktopBridge {
    computers(): Promise<Computers & { home: string }>;
    open(url: string): Promise<void>;
  }

  let { computer, remote, onRemoteAccess }: { computer: string; remote: RemoteAccess; onRemoteAccess: () => void } = $props();

  // Only the desktop app can switch computers; in a browser this is just the way to the remote access settings.
  const desktop = (globalThis as { linkerEditDesktop?: DesktopBridge }).linkerEditDesktop;
  let open = $state(false);
  let list = $state<(Computers & { home: string }) | null>(null);
  let failed = $state(false);
  let root: HTMLDivElement;

  const here = location.origin;
  const away = $derived(list !== null && new URL(list.home).origin !== here);
  const entries = $derived(list ? [{ name: list.name, url: list.home, home: true }, ...list.others.map((other) => ({ ...other, home: false }))] : []);

  async function load() {
    if (!desktop) return;
    failed = false;
    try {
      list = await desktop.computers();
    } catch {
      failed = true;
    }
  }

  onMount(() => void load());

  function toggle() {
    open = !open;
    if (open) void load();
  }

  function closeOnOutside(event: PointerEvent) {
    if (open && !root.contains(event.target as Node)) open = false;
  }
</script>

<svelte:window onpointerdown={closeOnOutside} onkeydown={(event) => event.key === "Escape" && (open = false)} />

<div class="relative mx-2 mb-1" bind:this={root}>
  <button
    type="button"
    class="flex h-8 w-full items-center gap-2 rounded-lg px-2.5 text-left text-xs transition-colors hover:bg-accent"
    aria-haspopup="menu"
    aria-expanded={open}
    title={desktop ? "Switch computer" : "Remote access"}
    onclick={toggle}
  >
    <Monitor class="size-3.5 shrink-0 text-muted-foreground" />
    <span class="min-w-0 flex-1 truncate font-medium">{computer}</span>
    {#if away}
      <span class="shrink-0 rounded border px-1 text-2xs text-muted-foreground">remote</span>
    {/if}
    {#if remote.state === "on"}
      <span class="size-1.5 shrink-0 rounded-full bg-success" title="Remote access is on"></span>
    {/if}
    <ChevronsUpDown class="size-3.5 shrink-0 text-muted-foreground" />
  </button>

  {#if open}
    <div class="absolute top-full right-0 left-0 z-50 mt-1 rounded-lg border bg-card p-1 shadow-lg shadow-black/10" role="menu">
      {#if desktop}
        <p class="px-2 pt-1 pb-1.5 text-2xs text-muted-foreground">Your computers</p>
        {#each entries as entry (entry.url)}
          {@const current = new URL(entry.url).origin === here}
          <button
            type="button"
            role="menuitem"
            class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors hover:bg-accent"
            onclick={() => {
              open = false;
              if (!current) void desktop.open(entry.url);
            }}
          >
            <Monitor class="size-3.5 shrink-0 text-muted-foreground" />
            <span class="min-w-0 flex-1 truncate">{entry.name}</span>
            {#if entry.home}<span class="shrink-0 text-2xs text-muted-foreground">this computer</span>{/if}
            {#if current}<Check class="size-3.5 shrink-0" />{/if}
          </button>
        {/each}
        {#if failed}
          <p class="px-2 py-1.5 text-xs text-destructive">Couldn't look for your computers.</p>
        {:else if !list}
          <p class="px-2 py-1.5 text-xs text-muted-foreground">Looking for your computers…</p>
        {:else if list.others.length === 0}
          <p class="px-2 py-1.5 text-xs text-muted-foreground">
            No other computers found. On the computer with Resolve, turn on remote access, and log in to the same Tailscale account on both.
          </p>
        {/if}
        <div class="my-1 border-t"></div>
      {/if}
      <button
        type="button"
        role="menuitem"
        class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors hover:bg-accent"
        onclick={() => {
          open = false;
          onRemoteAccess();
        }}
      >
        <Globe class="size-3.5 shrink-0" />
        Remote access{away ? ` on ${computer}` : ""}…
      </button>
    </div>
  {/if}
</div>
