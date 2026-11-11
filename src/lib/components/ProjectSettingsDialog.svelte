<script lang="ts">
  import { untrack } from "svelte";
  import type { CloudSyncMode, FolderListing, ProjectSummary } from "#shared/protocol.ts";
  import { trimSeparators } from "#lib/browsePath.ts";
  import { LIBRARY_LABEL, SYNC_MODES } from "#lib/labels.ts";
  import Dialog from "./Dialog.svelte";
  import FolderPicker from "./FolderPicker.svelte";

  let {
    project,
    relinkTo,
    browseFolders,
    onSave,
    onRelink,
    onRemove,
    onClose,
  }: {
    project: ProjectSummary;
    /** The project open in Resolve when no Linker-Edit project has it, to link this one to. */
    relinkTo: string | null;
    browseFolders: (path: string) => Promise<FolderListing | { error: string }>;
    onSave: (changes: { name?: string; mediaPath?: string; syncMode?: CloudSyncMode }) => void;
    onRelink: () => void;
    onRemove: () => void;
    onClose: () => void;
  } = $props();

  // A form edits a copy; the props are only its starting values.
  const initial = untrack(() => project);
  // Empty while the project shows its Resolve project's name, so a rename in Resolve still shows.
  const initialName = initial.name === initial.link?.name ? "" : initial.name;
  let name = $state(initialName);
  let mediaPath = $state(initial.local.mediaPath ?? "");
  let syncMode = $state<CloudSyncMode>(initial.local.syncMode ?? "proxy_only");

  const cloud = $derived(project.link?.kind === "cloud");

  function save() {
    onSave({ ...(name.trim() === initialName ? {} : { name }), ...(cloud ? { mediaPath: trimSeparators(mediaPath), syncMode } : {}) });
    onClose();
  }

  function remove() {
    if (confirm(`Remove "${project.name}" from Linker-Edit? Its folder, edit history and Resolve project stay; you can add it again.`)) {
      onRemove();
      onClose();
    }
  }

  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
</script>

<Dialog title="Project settings" {onClose}>
  <label class="flex flex-col gap-1 text-xs text-muted-foreground">
    Name
    <input class={field} placeholder={project.link?.name ?? project.name} bind:value={name} />
  </label>

  <div class="flex flex-col gap-1 text-xs">
    <span class="text-muted-foreground">Resolve project</span>
    {#if project.link}
      <p class="text-sm">
        {project.link.name}
        <span class="text-muted-foreground">· {LIBRARY_LABEL[project.link.kind]}{project.local.database ? ` "${project.local.database.name}"` : ""}</span>
      </p>
    {:else}
      <p class="text-sm text-muted-foreground">Not linked yet. The first message you send links it to the project open in Resolve.</p>
    {/if}
    {#if relinkTo}
      <button
        type="button"
        class="mt-1 h-7 self-start rounded-md border px-2.5 text-xs transition-colors hover:bg-accent"
        onclick={() => {
          onRelink();
          onClose();
        }}
      >
        Link to "{relinkTo}", open in Resolve now
      </button>
    {/if}
  </div>

  {#if cloud}
    <div class="flex flex-col gap-1 text-xs text-muted-foreground">
      Media folder on this computer
      <FolderPicker bind:value={mediaPath} browse={browseFolders} listOnFocus label="Media folder" />
    </div>
    <label class="flex flex-col gap-1 text-xs text-muted-foreground">
      Sync
      <select class={field} bind:value={syncMode}>
        {#each SYNC_MODES as mode (mode.value)}
          <option value={mode.value}>{mode.label}</option>
        {/each}
      </select>
    </label>
  {/if}

  <div class="flex flex-col gap-1 text-xs">
    <span class="text-muted-foreground">Folder</span>
    <p class="font-mono text-2xs break-all select-all">{project.dir}</p>
  </div>

  <div class="flex items-center gap-2 border-t pt-4">
    <button type="button" class="h-8 rounded-md px-2.5 text-sm text-destructive transition-colors hover:bg-destructive/10" onclick={remove}>
      Remove from Linker-Edit
    </button>
    <button
      type="button"
      class="ml-auto h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90"
      onclick={save}
    >
      Save
    </button>
  </div>
</Dialog>
