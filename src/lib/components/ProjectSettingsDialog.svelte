<script lang="ts">
  import { untrack } from "svelte";
  import { Cloud, HardDrive, Server } from "@lucide/svelte";
  import type { CloudSyncMode, CollectionSummary, FolderListing, ProjectIcon as Icon, ProjectSummary } from "#shared/protocol.ts";
  import { trimSeparators } from "#lib/browsePath.ts";
  import { COLLECTION_LABEL, COLLECTION_PICKER_LABEL, LIBRARY_LABEL, NO_COLLECTION, SYNC_MODES } from "#lib/labels.ts";
  import Dialog from "./Dialog.svelte";
  import FolderPicker from "./FolderPicker.svelte";
  import IconPicker from "./sidebar/IconPicker.svelte";
  import ProjectIcon from "./sidebar/ProjectIcon.svelte";

  let {
    project,
    collections,
    relinkTo,
    browseFolders,
    onSave,
    onMove,
    onRelink,
    onRemove,
    onClose,
  }: {
    project: ProjectSummary;
    /** Every workspace and series, to put the project in one. */
    collections: CollectionSummary[];
    /** The project open in Resolve when no Linker-Edit project has it, to link this one to. */
    relinkTo: string | null;
    browseFolders: (path: string) => Promise<FolderListing | { error: string }>;
    onSave: (changes: { name?: string; mediaPath?: string; syncMode?: CloudSyncMode; icon?: Icon | null }) => void;
    /** Puts the project in a workspace or series, or in none. */
    onMove: (collectionId: string | null) => void;
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
  let icon = $state<Icon | null>(initial.icon);
  let picking = $state(false);
  const KIND_ICON = { local: HardDrive, network: Server, cloud: Cloud };
  const initialCollection = untrack(() => collections.find((collection) => collection.projectIds.includes(initial.id))?.id ?? "");
  let collectionId = $state(initialCollection);

  // Workspaces, each followed by its series, then series on their own.
  const collectionOptions = $derived.by(() => {
    const workspaces = collections.filter((collection) => collection.kind === "workspace");
    const series = (parentId: string | null) =>
      collections.filter((collection) => collection.kind === "series" && collection.parentId === parentId);
    return [
      ...workspaces.flatMap((workspace) => [
        { id: workspace.id, label: workspace.name },
        ...series(workspace.id).map((entry) => ({ id: entry.id, label: `${workspace.name} / ${entry.name}` })),
      ]),
      ...series(null).map((entry) => ({ id: entry.id, label: `${entry.name} (${COLLECTION_LABEL.series.lower})` })),
    ];
  });

  const cloud = $derived(project.link?.kind === "cloud");

  function save() {
    onSave({
      ...(name.trim() === initialName ? {} : { name }),
      ...(cloud ? { mediaPath: trimSeparators(mediaPath), syncMode } : {}),
      ...(JSON.stringify(icon) === JSON.stringify(initial.icon) ? {} : { icon }),
    });
    if (collectionId !== initialCollection) onMove(collectionId || null);
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
  <div class="flex items-end gap-3">
    <div class="flex flex-col gap-1 text-xs text-muted-foreground">
      Icon
      <button
        type="button"
        class="flex size-8 items-center justify-center rounded-md border transition-colors hover:bg-accent"
        aria-label="Choose project icon"
        title="Choose project icon"
        onclick={() => (picking = true)}
      >
        <ProjectIcon {icon} name={name.trim() || project.name} class="size-5" />
      </button>
    </div>
    <label class="flex min-w-0 flex-1 flex-col gap-1 text-xs text-muted-foreground">
      Name
      <input class={field} placeholder={project.link?.name ?? project.name} bind:value={name} />
    </label>
  </div>
  {#if icon}
    <button type="button" class="-mt-2 self-start text-2xs text-muted-foreground hover:text-foreground hover:underline" onclick={() => (icon = null)}>
      Use the letters of its name
    </button>
  {/if}

  <label class="flex flex-col gap-1 text-xs text-muted-foreground">
    {COLLECTION_PICKER_LABEL}
    <select class={field} bind:value={collectionId}>
      <option value="">{NO_COLLECTION}</option>
      {#each collectionOptions as option (option.id)}
        <option value={option.id}>{option.label}</option>
      {/each}
    </select>
    {#if !collections.length}
      <span>Make {COLLECTION_LABEL.workspace.lowerMany} and {COLLECTION_LABEL.series.lowerMany} in Settings.</span>
    {/if}
  </label>

  <div class="flex flex-col gap-1 text-xs">
    <span class="text-muted-foreground">Resolve project</span>
    {#if project.link}
      {@const KindIcon = KIND_ICON[project.link.kind]}
      <p class="flex items-center gap-1.5 text-sm">
        <KindIcon class="size-3.5 shrink-0 text-muted-foreground" />
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

{#if picking}
  <IconPicker current={icon} projectName={name.trim() || project.name} onSelect={(next) => (icon = next)} onClose={() => (picking = false)} />
{/if}
