<script lang="ts">
  // A folder field for a form: shows the folder chosen, by name and path, and opens PathPickerDialog to choose
  // or change it. Nothing to type, so nothing to mistype.
  import { onMount, tick } from "svelte";
  import Folder from "@lucide/svelte/icons/folder";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import X from "@lucide/svelte/icons/x";
  import type { FolderListing } from "#shared/protocol.ts";
  import PathPickerDialog from "./PathPickerDialog.svelte";

  let {
    value = $bindable(""),
    open = $bindable(false),
    label,
    title,
    placeholder = "Choose a folder…",
    clearable = false,
    browse,
  }: {
    value?: string;
    /** Whether the picker is open; a form can open it at once, such as when a folder is all it's for. */
    open?: boolean;
    label: string;
    /** The picker's title. */
    title?: string;
    placeholder?: string;
    /** Shows a button to empty the field, for an optional folder. */
    clearable?: boolean;
    browse: (path: string) => Promise<FolderListing | { error: string }>;
  } = $props();

  const path = $derived(value.trim());
  const name = $derived(path.split(/[\\/]/).filter(Boolean).at(-1) ?? path);
  const dialogTitle = $derived(title ?? `Choose: ${label}`);

  // A field opened at once inside another dialog waits for that dialog to open first: the modal opened last
  // goes on top, and children open theirs before their parents do.
  let ready = $state(false);
  onMount(() => void tick().then(() => (ready = true)));
</script>

<div class="flex items-stretch gap-1.5">
  <button
    type="button"
    class="group flex min-h-10 min-w-0 flex-1 items-center gap-2.5 rounded-md border px-2.5 py-1.5 text-left transition-colors hover:border-primary/60 hover:bg-accent/40 focus-visible:border-primary focus-visible:outline-none"
    aria-label={path ? `${label}: ${path}. Change` : `${label}: choose a folder`}
    aria-haspopup="dialog"
    onclick={() => (open = true)}
  >
    {#if path}
      <Folder class="size-4 shrink-0 text-primary" />
      <span class="flex min-w-0 flex-1 flex-col">
        <span class="truncate text-sm">{name}</span>
        <span class="truncate font-mono text-2xs text-muted-foreground" title={path}>{path}</span>
      </span>
      <span class="shrink-0 text-xs text-muted-foreground group-hover:text-foreground">Change</span>
    {:else}
      <FolderOpen class="size-4 shrink-0 text-muted-foreground" />
      <span class="flex-1 text-sm text-muted-foreground group-hover:text-foreground">{placeholder}</span>
    {/if}
  </button>
  {#if clearable && path}
    <button
      type="button"
      class="flex w-8 shrink-0 items-center justify-center rounded-md border text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      aria-label={`Clear ${label.toLowerCase()}`}
      title="Clear"
      onclick={() => (value = "")}
    >
      <X class="size-3.5" />
    </button>
  {/if}
</div>

{#if open && ready}
  <PathPickerDialog title={dialogTitle} {label} start={path} {browse} onPick={(picked) => (value = picked)} onClose={() => (open = false)} />
{/if}
