<script lang="ts">
  // Adds one folder to a library, or edits one. A folder of folders, such as video-editing/ with Music/,
  // SoundEffects/ and VideoFX/ in it, is sorted by the server into one library folder each, with a kind guessed
  // from its name or its files; the user checks the kinds before they're added.
  import { untrack } from "svelte";
  import CircleHelp from "@lucide/svelte/icons/circle-help";
  import {
    LIBRARY_KINDS,
    type FolderListing,
    type LibraryFolder,
    type LibraryKind,
    type LibrarySort,
    type LibrarySortedFolder,
  } from "#shared/protocol.ts";
  import { trimSeparators } from "#lib/browsePath.ts";
  import { describeCounts, KIND_LABEL, samePath } from "#lib/library.ts";
  import { KIND_ICON } from "#lib/libraryIcons.ts";
  import Dialog from "./Dialog.svelte";
  import PathField from "./PathField.svelte";

  let {
    folder,
    where,
    existing = [],
    browseFolders,
    sortFolder,
    onSave,
    onSaveMany,
    onClose,
  }: {
    /** The folder to edit; null to add one. */
    folder: LibraryFolder | null;
    /** Which library, as the dialog says it: "the universal library". */
    where: string;
    /** The paths of the folders in this library already, which aren't offered again. */
    existing?: string[];
    browseFolders: (path: string) => Promise<FolderListing | { error: string }>;
    sortFolder?: (path: string) => Promise<LibrarySort | { error: string }>;
    onSave: (fields: Omit<LibraryFolder, "id">) => void;
    onSaveMany?: (folders: Omit<LibraryFolder, "id">[]) => void;
    onClose: () => void;
  } = $props();

  // A form edits a copy; the props are only its starting values.
  const initial = untrack(() => folder);
  let kind = $state<LibraryKind>(initial?.kind ?? "music");
  let path = $state(initial?.path ?? "");
  let label = $state(initial?.label ?? "");
  let note = $state(initial?.note ?? "");
  // Adding starts with the folder: it's what the dialog is for, and the name comes from it.
  let choosing = $state(!initial);

  const picked = $derived(trimSeparators(path.trim()));
  const fallbackLabel = $derived(picked.split(/[\\/]/).filter(Boolean).at(-1) ?? "");
  const canSave = $derived(!!picked);

  // --- Sorting a folder of folders ---
  let sort = $state<{ path: string; result: LibrarySort | { error: string } } | null>(null);
  let mode = $state<"one" | "many">("one");
  /** The kind chosen for each folder inside, by path; "" leaves it out. */
  let choices = $state<Record<string, LibraryKind | "">>({});

  const sorted = $derived(sort?.path === picked && !("error" in sort.result) ? sort.result : null);
  const sorting = $derived(!initial && !!sortFolder && !!picked && sort?.path !== picked);
  const isAdded = (candidate: LibrarySortedFolder) => existing.some((other) => samePath(other, candidate.path));
  const chosen = $derived(sorted ? sorted.folders.filter((candidate) => !isAdded(candidate) && choices[candidate.path]) : []);
  const unsure = $derived(sorted ? sorted.folders.filter((candidate) => !isAdded(candidate) && !candidate.kind).length : 0);
  /** Sorting is offered when the folder's own folders say what they are. */
  const sortable = $derived(!!sorted && sorted.folders.some((candidate) => candidate.reason === "name"));

  $effect(() => {
    const target = picked;
    if (initial || !sortFolder || !target) return;
    let current = true;
    void sortFolder(target).then((result) => {
      if (!current) return;
      sort = { path: target, result };
      if ("error" in result) return;
      choices = Object.fromEntries(result.folders.map((candidate) => [candidate.path, isAdded(candidate) ? "" : (candidate.kind ?? "")]));
      mode = result.folders.some((candidate) => candidate.reason === "name") ? "many" : "one";
      if (mode === "one" && result.rootKind) kind = result.rootKind;
    });
    return () => (current = false);
  });

  function save() {
    if (mode === "many" && sorted) {
      if (!chosen.length) return;
      onSaveMany?.(chosen.map((candidate) => ({ kind: choices[candidate.path] as LibraryKind, path: candidate.path, label: candidate.name.split("/").at(-1)!, note: "" })));
      onClose();
      return;
    }
    if (!canSave) return;
    onSave({ kind, path: picked, label: label.trim() || fallbackLabel, note: note.trim() });
    onClose();
  }

  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
  const linkButton = "text-xs text-muted-foreground underline-offset-2 transition-colors hover:text-foreground hover:underline";
</script>

<Dialog title={folder ? "Edit folder" : "Add a folder"} width={mode === "many" && sorted ? "w-[36rem]" : "w-[30rem]"} {onClose}>
  <p class="-mt-1 text-xs text-muted-foreground">
    {folder ? "In" : "To"}
    {where}. Linker-Edit only points at the folder: your files stay where they are.
  </p>

  <div class="flex flex-col gap-1 text-xs text-muted-foreground">
    Folder
    <PathField bind:value={path} bind:open={choosing} label="Library folder" title="Choose a library folder" browse={browseFolders} />
    {#if sorting}
      <span>Looking at what's inside…</span>
    {:else if sort?.path === picked && "error" in sort.result}
      <span class="text-destructive">{sort.result.error}</span>
    {/if}
  </div>

  {#if mode === "many" && sorted}
    <div class="flex flex-col gap-2">
      <p class="text-sm">
        It has {sorted.folders.length}{sorted.more ? "+" : ""} folders inside. Check what each one is: everything inside a folder comes along.
      </p>
      <ul class="-mx-1 flex max-h-80 flex-col overflow-y-auto" aria-label="Folders inside">
        {#each sorted.folders as candidate (candidate.path)}
          {@const added = isAdded(candidate)}
          {@const choice = choices[candidate.path] ?? ""}
          {@const Icon = choice ? KIND_ICON[choice] : CircleHelp}
          <li class="flex items-center gap-2.5 rounded-md px-2 py-1.5 {added ? 'opacity-50' : 'hover:bg-accent/40'}">
            <Icon class="size-4 shrink-0 {choice ? 'text-primary' : 'text-muted-foreground'}" />
            <div class="flex min-w-0 flex-1 flex-col">
              <span class="truncate text-sm" title={candidate.path}>{candidate.name}</span>
              <span class="truncate text-2xs text-muted-foreground">
                {describeCounts(candidate.counts, candidate.more)}
                {#if added}
                  · already in the library
                {:else if candidate.reason === "name"}
                  · by its name
                {:else if candidate.reason === "files"}
                  · by its files
                {:else}
                  · <span class="text-amber-600 dark:text-amber-400">not sure, pick one</span>
                {/if}
              </span>
            </div>
            <select
              class="h-7 w-36 shrink-0 rounded-md border bg-transparent px-1.5 text-xs outline-none focus:border-primary disabled:opacity-60"
              aria-label="Kind of {candidate.name}"
              disabled={added}
              bind:value={() => choice, (value) => (choices[candidate.path] = value)}
            >
              <option value="">Leave out</option>
              {#each LIBRARY_KINDS as option (option)}
                <option value={option}>{KIND_LABEL[option]}</option>
              {/each}
            </select>
          </li>
        {/each}
      </ul>
      {#if unsure}
        <p class="text-xs text-muted-foreground">
          {unsure === 1 ? "One folder is" : `${unsure} folders are`} left out until you pick a kind.
        </p>
      {/if}
    </div>
  {:else}
    <label class="flex flex-col gap-1 text-xs text-muted-foreground">
      Kind
      <select class={field} bind:value={kind}>
        {#each LIBRARY_KINDS as option (option)}
          <option value={option}>{KIND_LABEL[option]}</option>
        {/each}
      </select>
    </label>

    <label class="flex flex-col gap-1 text-xs text-muted-foreground">
      Name
      <input class={field} placeholder={fallbackLabel || "Epidemic downloads"} bind:value={label} />
    </label>

    <label class="flex flex-col gap-1 text-xs text-muted-foreground">
      Note for you and the agents
      <textarea
        class="min-h-16 w-full rounded-md border bg-transparent px-2.5 py-1.5 text-sm outline-none focus:border-primary"
        placeholder="Licensed for YouTube only. Use for intros."
        bind:value={note}
      ></textarea>
    </label>
  {/if}

  <div class="flex items-center gap-2 border-t pt-4">
    {#if mode === "many" && sorted}
      <button type="button" class={linkButton} onclick={() => (mode = "one")}>Add it as one folder instead</button>
    {:else if sortable}
      <button type="button" class={linkButton} onclick={() => (mode = "many")}>Sort the folders inside instead</button>
    {/if}
    <button type="button" class="ml-auto h-8 rounded-md px-3 text-sm transition-colors hover:bg-accent" onclick={onClose}>Cancel</button>
    <button
      type="button"
      class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
      disabled={mode === "many" && sorted ? !chosen.length : !canSave}
      onclick={save}
    >
      {#if folder}
        Save
      {:else if mode === "many" && sorted}
        Add {chosen.length} {chosen.length === 1 ? "folder" : "folders"}
      {:else}
        Add folder
      {/if}
    </button>
  </div>
</Dialog>
