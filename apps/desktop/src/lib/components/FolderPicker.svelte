<script lang="ts">
  // A path field that lists the folders of the server's computer as you type, like T3 Code's
  // "Add project": ↑/↓ choose, Enter or Tab goes into a folder, Enter on the path itself picks it.
  import { onMount, tick, untrack } from "svelte";
  import CornerLeftUp from "@lucide/svelte/icons/corner-left-up";
  import FileIcon from "@lucide/svelte/icons/file";
  import FileImage from "@lucide/svelte/icons/file-image";
  import FileMusic from "@lucide/svelte/icons/file-music";
  import FileVideo from "@lucide/svelte/icons/file-video-camera";
  import Folder from "@lucide/svelte/icons/folder";
  import FolderCheck from "@lucide/svelte/icons/folder-check";
  import FolderSearch from "@lucide/svelte/icons/folder-search";
  import House from "@lucide/svelte/icons/house";
  import type { FolderEntry, FolderListing } from "#shared/protocol.ts";
  import {
    appendSegment,
    browseDirectory,
    browseFilter,
    fileKind,
    filterEntries,
    filterFiles,
    hasTrailingSeparator,
    parentDirectory,
    tabCompletion,
    trimSeparators,
  } from "#lib/browsePath.ts";
  import { desktop } from "#lib/desktop.ts";
  import { useHost } from "#lib/hostContext.ts";

  type Reply = FolderListing | { error: string };

  let {
    value = $bindable(""),
    status = $bindable(null),
    browse,
    placeholder = "",
    autofocus = false,
    label,
    native,
    onSubmit,
    onPicked,
  }: {
    value?: string;
    /** What's typed: an existing folder, and whether it holds a linker-edit.json. Null while it's being checked. */
    status?: { exists: boolean; isProject: boolean } | null;
    browse: (path: string) => Promise<Reply>;
    placeholder?: string;
    autofocus?: boolean;
    label?: string;
    /** Whether the folders are this computer's, so the system's folder dialog may choose one. By default, the provided host's. */
    native?: boolean;
    /** Enter on the path itself, with no folder in the list chosen. */
    onSubmit?: (path: string) => void;
    /** A folder chosen in the system's dialog; without it, the dialog only fills in the field. */
    onPicked?: (path: string) => void;
  } = $props();

  const DEBOUNCE_MS = 100;
  const listId = $props.id();
  let input: HTMLInputElement;
  let listElement = $state<HTMLElement>();
  let cache = $state(new Map<string, Reply>());
  /** The last good listing, for the drive shortcuts while another folder loads. */
  let lastListing = $state<FolderListing | null>(null);
  let highlight = $state(-1);
  const host = useHost();
  let desktopPicks = $state(false);
  // The system's dialog shows this computer's folders, so only for a machine that's this computer.
  const canPick = $derived(desktopPicks && (native ?? host().local));

  const directory = $derived(browseDirectory(value.trim()));
  const filter = $derived(browseFilter(value.trim()));
  const reply = $derived(directory ? cache.get(directory) : undefined);
  const listing = $derived(reply && !("error" in reply) ? reply : null);
  const visible = $derived(listing ? filterEntries(listing.entries, filter) : []);
  const parent = $derived(listing && !filter ? parentDirectory(listing.dir) : null);
  type Row = { kind: "up"; path: string } | { kind: "entry"; entry: FolderEntry };
  const rows = $derived<Row[]>([...(parent ? [{ kind: "up" as const, path: parent }] : []), ...visible.map((entry) => ({ kind: "entry" as const, entry }))]);
  /** Shown greyed out after the folders, so you can see what a folder holds; they can't be chosen. */
  const files = $derived(listing ? filterFiles(listing.files ?? [], filter) : []);
  const FILES_SHOWN = 200;
  const FILE_ICON = { video: FileVideo, audio: FileMusic, image: FileImage, other: FileIcon };

  async function load(path: string): Promise<Reply> {
    const result = await browse(path);
    cache = new Map(cache).set(path, result);
    if (!("error" in result)) {
      lastListing = result;
      // The folder as the server spells it, so going up from it works without asking again.
      if (result.dir !== path) cache = new Map(cache).set(result.dir, result);
    }
    return result;
  }

  onMount(() => {
    if (autofocus) input.focus();
    void desktop?.canPickFolder?.().then((can) => (desktopPicks = can));
    // Starts an empty field in the home folder, written out the way the server's computer writes paths.
    if (untrack(() => value.trim())) return;
    void load("").then((result) => {
      if (!("error" in result) && !value.trim()) value = result.dir;
    });
  });

  $effect(() => {
    const path = directory;
    if (!path || untrack(() => cache.has(path))) return;
    const timer = setTimeout(() => void load(path), DEBOUNCE_MS);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    // A typed filter chooses its first match, so Enter goes into it; a bare folder path chooses nothing.
    void value;
    highlight = filter && visible.length ? rows.findIndex((row) => row.kind === "entry") : -1;
  });

  $effect(() => {
    if (!directory || !reply) status = directory ? null : { exists: false, isProject: false };
    else if (!listing) status = { exists: false, isProject: false };
    else if (!filter) status = { exists: true, isProject: listing.isProject };
    else {
      const exact = listing.entries.find((entry) => entry.name.toLowerCase() === filter.toLowerCase());
      status = { exists: Boolean(exact), isProject: exact?.isProject ?? false };
    }
  });

  async function setValue(next: string) {
    value = next;
    await tick();
    input.focus();
    input.setSelectionRange(next.length, next.length);
  }

  function activate(row: Row) {
    void setValue(row.kind === "up" ? row.path : appendSegment(value.trim(), row.entry.name));
  }

  function scrollToHighlight() {
    void tick().then(() => listElement?.querySelector(`[data-row="${highlight}"]`)?.scrollIntoView({ block: "nearest" }));
  }

  function onkeydown(event: KeyboardEvent) {
    if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      if (!rows.length) return;
      event.preventDefault();
      highlight = event.key === "ArrowDown" ? Math.min(rows.length - 1, highlight + 1) : Math.max(-1, highlight - 1);
      scrollToHighlight();
    } else if (event.key === "Enter") {
      event.preventDefault();
      const row = rows[highlight];
      if (row && !event.ctrlKey && !event.metaKey) activate(row);
      else if (value.trim()) onSubmit?.(trimSeparators(value));
    } else if (event.key === "Tab" && !event.shiftKey) {
      const completion = filter ? tabCompletion(visible, filter) : null;
      const row = rows[highlight];
      if (completion) {
        event.preventDefault();
        void setValue(completion.complete ? appendSegment(value.trim(), completion.name) : `${directory}${completion.name}`);
      } else if (row) {
        event.preventDefault();
        activate(row);
      }
    }
  }

  async function pickWithSystemDialog() {
    const picked = await desktop?.pickFolder?.(directory ? trimSeparators(directory) : undefined);
    if (!picked) return;
    if (onPicked) return onPicked(trimSeparators(picked));
    void setValue(hasTrailingSeparator(picked) ? picked : `${picked}${picked.includes("\\") ? "\\" : "/"}`);
  }

  const shortcuts = $derived(lastListing ? [{ label: "Home", path: lastListing.home, home: true }, ...lastListing.roots.map((root) => ({ label: root, path: root, home: false }))] : []);
  // Keeps the field focused when a row or shortcut is clicked, so typing and the arrow keys carry on.
  const keepFocus = (event: MouseEvent) => event.preventDefault();
</script>

<div class="flex flex-col gap-2">
  <div class="flex gap-1.5">
    <input
      bind:this={input}
      bind:value
      {placeholder}
      aria-label={label}
      class="h-8 min-w-0 flex-1 rounded-md border bg-transparent px-2.5 font-mono text-xs outline-none focus:border-primary"
      spellcheck="false"
      autocomplete="off"
      role="combobox"
      aria-expanded="true"
      aria-controls={listId}
      aria-autocomplete="list"
      {onkeydown}
    />
    {#if canPick}
      <button
        type="button"
        class="flex h-8 shrink-0 items-center gap-1.5 rounded-md border px-2.5 text-xs text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="Choose with the system's folder dialog"
        onclick={pickWithSystemDialog}
      >
        <FolderSearch class="size-3.5" />
        Browse…
      </button>
    {/if}
  </div>

  {#if shortcuts.length > 1 || (shortcuts.length && !shortcuts[0]!.home)}
    <div class="flex flex-wrap gap-1">
      {#each shortcuts as shortcut (shortcut.path)}
        <button
          type="button"
          class="flex h-6 items-center gap-1 rounded-md bg-accent px-2 text-2xs text-muted-foreground transition-colors hover:text-foreground"
          onmousedown={keepFocus}
          onclick={() => void setValue(shortcut.path)}
        >
          {#if shortcut.home}<House class="size-3" />{/if}
          {shortcut.label}
        </button>
      {/each}
    </div>
  {/if}

  <div bind:this={listElement} id={listId} class="-mx-1 h-64 overflow-y-auto" role="listbox" aria-label="Folders">
    {#if reply && "error" in reply}
      <p class="px-2 py-4 text-center text-xs text-destructive">{reply.error}</p>
    {:else if !directory}
      <p class="px-2 py-4 text-center text-xs text-muted-foreground">Start the path with a drive, / or ~.</p>
    {:else if !listing}
      <p class="px-2 py-4 text-center text-xs text-muted-foreground">Looking in {directory}…</p>
    {:else}
      {#each rows as row, index (row.kind === "up" ? ".." : row.entry.name)}
        <button
          type="button"
          role="option"
          aria-selected={index === highlight}
          data-row={index}
          class="flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-sm transition-colors {index === highlight
            ? 'bg-accent text-foreground'
            : 'hover:bg-accent/60'}"
          onmousedown={keepFocus}
          onmousemove={() => (highlight = index)}
          onclick={() => activate(row)}
        >
          {#if row.kind === "up"}
            <CornerLeftUp class="size-3.5 shrink-0 text-muted-foreground" />
            <span class="text-muted-foreground">..</span>
          {:else}
            {#if row.entry.isProject}
              <FolderCheck class="size-3.5 shrink-0 text-primary" />
            {:else}
              <Folder class="size-3.5 shrink-0 text-muted-foreground" />
            {/if}
            <span class="truncate">{row.entry.name}</span>
            {#if row.entry.isProject}
              <span class="ml-auto shrink-0 rounded bg-primary/15 px-1.5 text-2xs text-primary">Linker-Edit project</span>
            {/if}
          {/if}
        </button>
      {/each}
      {#each files.slice(0, FILES_SHOWN) as file (file)}
        {@const Icon = FILE_ICON[fileKind(file)]}
        <div class="flex items-center gap-2 px-2 py-1 text-sm text-muted-foreground/60" title="Files can't be chosen here, only folders">
          <Icon class="size-3.5 shrink-0" />
          <span class="truncate">{file}</span>
        </div>
      {/each}
      {#if files.length > FILES_SHOWN}
        <p class="px-2 py-1 text-xs text-muted-foreground/60">and {files.length - FILES_SHOWN} more files</p>
      {/if}
      {#if !visible.length && !files.length}
        <p class="px-2 py-4 text-center text-xs text-muted-foreground">
          {filter ? `Nothing here starts with "${filter}".` : "This folder is empty."}
        </p>
      {/if}
    {/if}
  </div>
  <p class="text-2xs text-muted-foreground">↑↓ choose a folder · Enter or Tab opens it · Enter with none chosen picks this path</p>
</div>
