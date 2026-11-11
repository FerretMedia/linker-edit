<script lang="ts">
  // A path field that lists the folders of the server's computer as you type, like T3 Code's
  // "Add project": ↑/↓ choose, Enter or Tab goes into a folder, Enter on the path itself picks it.
  import { onMount, tick, untrack } from "svelte";
  import { CornerLeftUp, Folder, FolderCheck, FolderSearch, House } from "@lucide/svelte";
  import type { FolderEntry, FolderListing } from "#shared/protocol.ts";
  import {
    appendSegment,
    browseDirectory,
    browseFilter,
    filterEntries,
    hasTrailingSeparator,
    parentDirectory,
    tabCompletion,
    trimSeparators,
  } from "#lib/browsePath.ts";
  import { desktop } from "#lib/desktop.ts";

  type Reply = FolderListing | { error: string };

  let {
    value = $bindable(""),
    status = $bindable(null),
    browse,
    placeholder = "",
    autofocus = false,
    listOnFocus = false,
    label,
    onSubmit,
  }: {
    value?: string;
    /** What's typed: an existing folder, and whether it holds a linker-edit.json. Null while it's being checked. */
    status?: { exists: boolean; isProject: boolean } | null;
    browse: (path: string) => Promise<Reply>;
    placeholder?: string;
    autofocus?: boolean;
    /** Shows the list only while the field has focus, for a form with other fields. */
    listOnFocus?: boolean;
    label?: string;
    /** Enter on the path itself, with no folder in the list chosen. */
    onSubmit?: (path: string) => void;
  } = $props();

  const DEBOUNCE_MS = 100;
  const listId = $props.id();
  let input: HTMLInputElement;
  let listElement = $state<HTMLElement>();
  let focused = $state(false);
  let cache = $state(new Map<string, Reply>());
  /** The last good listing, for the drive shortcuts while another folder loads. */
  let lastListing = $state<FolderListing | null>(null);
  let highlight = $state(-1);
  let canPick = $state(false);

  const directory = $derived(browseDirectory(value.trim()));
  const filter = $derived(browseFilter(value.trim()));
  const reply = $derived(directory ? cache.get(directory) : undefined);
  const listing = $derived(reply && !("error" in reply) ? reply : null);
  const visible = $derived(listing ? filterEntries(listing.entries, filter) : []);
  const parent = $derived(listing && !filter ? parentDirectory(listing.dir) : null);
  type Row = { kind: "up"; path: string } | { kind: "entry"; entry: FolderEntry };
  const rows = $derived<Row[]>([...(parent ? [{ kind: "up" as const, path: parent }] : []), ...visible.map((entry) => ({ kind: "entry" as const, entry }))]);

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
    void desktop?.canPickFolder?.().then((can) => (canPick = can));
  });

  let started = false;
  $effect(() => {
    // Starts an empty field in the home folder once the list shows, written out the way the server's
    // computer writes paths. Not before: a field that only lists on focus shouldn't fill itself in.
    if (!showList || started) return;
    started = true;
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
    void setValue(hasTrailingSeparator(picked) ? picked : `${picked}${picked.includes("\\") ? "\\" : "/"}`);
  }

  const shortcuts = $derived(lastListing ? [{ label: "Home", path: lastListing.home, home: true }, ...lastListing.roots.map((root) => ({ label: root, path: root, home: false }))] : []);
  const showList = $derived(!listOnFocus || focused);
  // Keeps the field focused when a row or shortcut is clicked, so typing and the arrow keys carry on.
  const keepFocus = (event: MouseEvent) => event.preventDefault();
</script>

<div
  class="flex flex-col gap-2"
  onfocusin={() => (focused = true)}
  onfocusout={(event) => {
    if (!event.currentTarget.contains(event.relatedTarget as Node | null)) focused = false;
  }}
>
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
      aria-expanded={showList}
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

  {#if showList}
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

    <div bind:this={listElement} id={listId} class="-mx-1 overflow-y-auto {listOnFocus ? 'h-40' : 'h-56'}" role="listbox" aria-label="Folders">
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
        {:else}
          <p class="px-2 py-4 text-center text-xs text-muted-foreground">
            {filter ? `No folder here starts with "${filter}".` : "No folders in here."}
          </p>
        {/each}
      {/if}
    </div>
    <p class="text-2xs text-muted-foreground">↑↓ choose a folder · Enter or Tab opens it · Enter with none chosen picks this path</p>
  {/if}
</div>
