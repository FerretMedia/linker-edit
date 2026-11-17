<script lang="ts">
  import type { AppUpdate } from "#shared/protocol.ts";
  import { RELEASES_URL, releaseName } from "#shared/release-name.ts";

  // The desktop app's own updates (server/appUpdate.ts). On another of the user's computers it's that computer's
  // app: restarting restarts Linker-Edit there.
  let {
    update,
    computer,
    remotePage,
    asked,
    onInstall,
    onCancel,
    onCheck,
  }: {
    update: AppUpdate;
    computer: string;
    /** The page is another computer's, over Tailscale. */
    remotePage: boolean;
    /** The user clicked "Check for updates": say how the check went, even when there's nothing new. */
    asked: boolean;
    onInstall: () => void;
    onCancel: () => void;
    onCheck: () => void;
  } = $props();

  const version = $derived(releaseName(update.downloadedVersion ?? update.availableVersion ?? ""));
  const failedBefore = $derived(update.notApplied !== null && update.notApplied === update.downloadedVersion);
  const restartLabel = $derived(remotePage ? `Restart ${computer} to update` : "Restart to update");

  // Background checks that fail, say offline, stay quiet; a failed download or install doesn't.
  const visible = $derived.by(() => {
    switch (update.status) {
      case "idle":
        return false;
      case "checking":
      case "up-to-date":
        return asked;
      case "error":
        return asked || update.availableVersion !== null;
      default:
        return true;
    }
  });
</script>

{#snippet notes()}
  {#if update.notes.length}
    <details class="mt-1 text-2xs text-muted-foreground">
      <summary class="cursor-pointer select-none hover:text-foreground">What's new</summary>
      <ul class="mt-1 list-disc space-y-0.5 pl-3.5">
        {#each update.notes as note, index (index)}
          <li>{note}</li>
        {/each}
      </ul>
    </details>
  {/if}
{/snippet}

{#if visible}
  <div class="mx-2 rounded-lg px-2.5 py-2 shadow-[0_0_0_1px_var(--border)]" role="status">
    {#if update.status === "checking"}
      <span class="block text-xs font-medium">Looking for updates…</span>
    {:else if update.status === "up-to-date"}
      <span class="block text-xs font-medium">Linker-Edit is up to date</span>
      <span class="mt-0.5 block text-2xs text-muted-foreground">{releaseName(update.currentVersion)} is the newest.</span>
    {:else if update.status === "error"}
      <span class="block text-xs font-medium">Couldn't update</span>
      <span class="mt-0.5 line-clamp-3 block text-2xs break-words text-muted-foreground" title={update.error ?? ""}>{update.error}</span>
      <button type="button" class="mt-0.5 text-2xs text-primary hover:underline" onclick={onCheck}>Try again</button>
    {:else if update.status === "available" && update.downloadUrl}
      <span class="block text-xs font-medium">Linker-Edit {version} is out</span>
      {@render notes()}
      <a class="mt-0.5 block text-2xs text-primary hover:underline" href={update.downloadUrl} target="_blank" rel="noreferrer">
        Download it
      </a>
    {:else if update.status === "available" || update.status === "downloading"}
      <span class="block text-xs font-medium">
        Downloading Linker-Edit {version}{update.percent !== null ? ` · ${update.percent}%` : "…"}
      </span>
      <div class="mt-1.5 h-1 overflow-hidden rounded-full bg-border">
        <div class="h-full rounded-full bg-primary transition-[width]" style:width="{update.percent ?? 0}%"></div>
      </div>
      {#if update.error}
        <span class="mt-1 line-clamp-2 block text-2xs text-muted-foreground" title={update.error}>
          The download stopped: {update.error} It tries again later.
        </span>
      {/if}
    {:else if update.status === "downloaded" && update.waiting}
      <span class="block text-xs font-medium">Updating when Resolve is free</span>
      <span class="mt-0.5 block text-2xs text-muted-foreground">
        Linker-Edit restarts into {version} once the running turn, render or open is done.
      </span>
      <button type="button" class="mt-0.5 text-2xs text-primary hover:underline" onclick={onCancel}>Not now</button>
    {:else if update.status === "downloaded"}
      {#if failedBefore}
        <span class="block text-xs font-medium">Linker-Edit {version} didn't install</span>
        <span class="mt-0.5 block text-2xs text-muted-foreground">
          The last restart came back on {releaseName(update.currentVersion)}.
        </span>
      {:else}
        <span class="block text-xs font-medium">Linker-Edit {version} is ready</span>
      {/if}
      {#if update.error}
        <span class="mt-0.5 line-clamp-3 block text-2xs break-words text-destructive" title={update.error}>{update.error}</span>
      {/if}
      {@render notes()}
      <div class="mt-0.5 flex items-center gap-3">
        <button type="button" class="text-2xs text-primary hover:underline" onclick={onInstall}>
          {failedBefore || update.error ? "Try again" : restartLabel}
        </button>
        {#if failedBefore}
          <a class="text-2xs text-primary hover:underline" href={RELEASES_URL} target="_blank" rel="noreferrer">Download it</a>
        {/if}
      </div>
    {/if}
  </div>
{/if}
