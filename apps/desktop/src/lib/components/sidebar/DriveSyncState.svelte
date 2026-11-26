<script lang="ts">
  // A project's Google Drive sync, as small as the sidebar's other project marks: up to date, syncing, or a problem.
  import CloudAlert from "@lucide/svelte/icons/cloud-alert";
  import CloudCheck from "@lucide/svelte/icons/cloud-check";
  import CloudSync from "@lucide/svelte/icons/cloud-sync";
  import type { DriveSync } from "#shared/protocol.ts";
  import { syncPercent, syncText } from "#lib/sidebar.ts";

  let {
    sync,
    class: muted,
  }: {
    sync: DriveSync;
    /** Its colour when nothing is wrong. */
    class: string;
  } = $props();

  const percent = $derived(syncPercent(sync));
</script>

<span class="flex shrink-0 items-center gap-0.5 {sync.state === 'error' ? 'text-warning-foreground' : muted}" data-testid="sidebar-project-sync">
  {#if sync.state === "synced"}
    <CloudCheck aria-hidden="true" class="size-3.5" />
  {:else if sync.state === "error"}
    <CloudAlert aria-hidden="true" class="size-3.5" />
  {:else}
    <CloudSync aria-hidden="true" class="size-3.5" />
  {/if}
  {#if percent !== null}
    <span aria-hidden="true" class="text-2xs tabular-nums">{percent}%</span>
  {/if}
  <span class="sr-only">, {syncText(sync)}</span>
</span>
