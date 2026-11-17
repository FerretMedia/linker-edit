<script lang="ts">
  import { untrack } from "svelte";
  import { LIBRARY_KINDS, type FolderListing, type LibraryFolder, type LibraryKind } from "#shared/protocol.ts";
  import { trimSeparators } from "#lib/browsePath.ts";
  import { KIND_LABEL } from "#lib/library.ts";
  import Dialog from "./Dialog.svelte";
  import FolderPicker from "./FolderPicker.svelte";

  let {
    folder,
    where,
    browseFolders,
    onSave,
    onClose,
  }: {
    /** The folder to edit; null to add one. */
    folder: LibraryFolder | null;
    /** Which library, as the dialog says it: "the universal library". */
    where: string;
    browseFolders: (path: string) => Promise<FolderListing | { error: string }>;
    onSave: (fields: Omit<LibraryFolder, "id">) => void;
    onClose: () => void;
  } = $props();

  // A form edits a copy; the props are only its starting values.
  const initial = untrack(() => folder);
  let kind = $state<LibraryKind>(initial?.kind ?? "music");
  let path = $state(initial?.path ?? "");
  let label = $state(initial?.label ?? "");
  let note = $state(initial?.note ?? "");
  let status = $state<{ exists: boolean; isProject: boolean } | null>(null);

  const picked = $derived(trimSeparators(path.trim()));
  const fallbackLabel = $derived(picked.split(/[\\/]/).filter(Boolean).at(-1) ?? "");
  const canSave = $derived(!!picked && status?.exists === true);

  function save() {
    if (!canSave) return;
    onSave({ kind, path: picked, label: label.trim() || fallbackLabel, note: note.trim() });
    onClose();
  }

  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
</script>

<Dialog title={folder ? "Edit folder" : "Add a folder"} {onClose}>
  <p class="-mt-1 text-xs text-muted-foreground">
    {folder ? "In" : "To"}
    {where}. Linker-Edit only points at the folder: your files stay where they are.
  </p>

  <label class="flex flex-col gap-1 text-xs text-muted-foreground">
    Kind
    <select class={field} bind:value={kind}>
      {#each LIBRARY_KINDS as option (option)}
        <option value={option}>{KIND_LABEL[option]}</option>
      {/each}
    </select>
  </label>

  <div class="flex flex-col gap-1 text-xs text-muted-foreground">
    Folder
    <FolderPicker bind:value={path} bind:status browse={browseFolders} listOnFocus label="Library folder" onSubmit={save} />
  </div>

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

  <div class="flex items-center justify-end gap-2 border-t pt-4">
    <button type="button" class="h-8 rounded-md px-3 text-sm transition-colors hover:bg-accent" onclick={onClose}>Cancel</button>
    <button
      type="button"
      class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
      disabled={!canSave}
      onclick={save}
    >
      {folder ? "Save" : "Add folder"}
    </button>
  </div>
</Dialog>
