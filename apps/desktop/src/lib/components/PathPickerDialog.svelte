<script lang="ts">
  // A popup to choose a folder on the server's computer: the same browser as "Add project", with a button
  // to choose. PathField opens it; the folder only counts once it's chosen.
  import { untrack } from "svelte";
  import type { FolderListing } from "#shared/protocol.ts";
  import { hasTrailingSeparator, trimSeparators } from "#lib/browsePath.ts";
  import Dialog from "./Dialog.svelte";
  import FolderPicker from "./FolderPicker.svelte";

  let {
    title,
    label,
    start = "",
    browse,
    onPick,
    onClose,
  }: {
    title: string;
    /** The path field's name, for screen readers and tests. */
    label: string;
    /** The folder chosen before, opened to show what's in it; empty starts in the home folder. */
    start?: string;
    browse: (path: string) => Promise<FolderListing | { error: string }>;
    onPick: (path: string) => void;
    onClose: () => void;
  } = $props();

  const initial = untrack(() => start.trim());
  let value = $state(initial && !hasTrailingSeparator(initial) ? `${initial}${initial.includes("\\") ? "\\" : "/"}` : initial);
  let status = $state<{ exists: boolean; isProject: boolean } | null>(null);

  const picked = $derived(trimSeparators(value.trim()));
  const name = $derived(picked.split(/[\\/]/).filter(Boolean).at(-1) ?? picked);
  const canPick = $derived(!!picked && status?.exists === true);

  function pick(path: string) {
    onPick(path);
    onClose();
  }

  function choose() {
    if (canPick) pick(picked);
  }
</script>

<Dialog {title} width="w-[38rem]" {onClose}>
  <!-- A folder from the system's own dialog is chosen at once: you already said which one there. -->
  <FolderPicker bind:value bind:status {browse} autofocus {label} onSubmit={choose} onPicked={pick} />

  <div class="flex items-center gap-2 border-t pt-4">
    <p class="min-w-0 flex-1 truncate text-xs text-muted-foreground" title={picked}>
      {#if canPick}
        Choose <span class="font-medium text-foreground">{name}</span>
      {:else if status === null}
        Looking…
      {:else}
        Open a folder to choose it.
      {/if}
    </p>
    <button type="button" class="h-8 rounded-md px-3 text-sm transition-colors hover:bg-accent" onclick={onClose}>Cancel</button>
    <button
      type="button"
      class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
      disabled={!canPick}
      onclick={choose}
    >
      Choose folder
    </button>
  </div>
</Dialog>
