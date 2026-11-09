<script lang="ts">
  import { untrack } from "svelte";
  import { Cloud, FolderInput, HardDrive, MonitorUp } from "@lucide/svelte";
  import type { CloudSyncMode, ProjectSource, ResolveLibrary, ResolveStatus } from "#shared/protocol.ts";
  import { LIBRARY_LABEL, SYNC_MODES } from "#lib/labels.ts";
  import Dialog from "./Dialog.svelte";

  let {
    resolve,
    library,
    onLoadLibrary,
    onAdd,
    onClose,
  }: {
    resolve: ResolveStatus;
    /** Null while it loads. */
    library: ResolveLibrary | { error: string } | null;
    onLoadLibrary: () => void;
    onAdd: (source: ProjectSource) => void;
    onClose: () => void;
  } = $props();

  const TABS = [
    { value: "open", label: "Open in Resolve", Icon: MonitorUp },
    { value: "library", label: "Database", Icon: HardDrive },
    { value: "cloud", label: "Cloud", Icon: Cloud },
    { value: "folder", label: "Folder", Icon: FolderInput },
  ] as const;

  const openProject = $derived(resolve.state === "connected" ? resolve.project : null);
  // Starts where the likely project is; the user picks the tab after that.
  let tab = $state<(typeof TABS)[number]["value"]>(untrack(() => (openProject ? "open" : "library")));
  let filter = $state("");
  let cloudName = $state("");
  let mediaPath = $state("");
  let syncMode = $state<CloudSyncMode>("proxy_only");
  let folder = $state("");

  $effect(() => {
    // Asks Resolve for its projects each time the tab opens, so the list is current.
    if (tab === "library") onLoadLibrary();
  });

  const projects = $derived.by(() => {
    if (!library || "error" in library) return [];
    const query = filter.trim().toLowerCase();
    return library.projects.filter((project) => !query || `${project.folder.join("/")}/${project.name}`.toLowerCase().includes(query));
  });

  const otherDatabases = $derived.by(() => {
    if (!library || "error" in library) return [];
    return library.databases.filter((db) => db.name !== library.database?.name || (db.host ?? "") !== (library.database?.host ?? ""));
  });

  function add(source: ProjectSource) {
    onAdd(source);
    onClose();
  }

  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
</script>

<Dialog title="Add project" {onClose}>
  <div class="flex gap-1 rounded-lg bg-accent p-1" role="tablist">
    {#each TABS as item (item.value)}
      <button
        type="button"
        role="tab"
        aria-selected={tab === item.value}
        class="flex h-7 flex-1 items-center justify-center gap-1.5 rounded-md text-xs transition-colors {tab === item.value
          ? 'bg-card text-foreground shadow-sm'
          : 'text-muted-foreground hover:text-foreground'}"
        onclick={() => (tab = item.value)}
      >
        <item.Icon class="size-3.5" />
        {item.label}
      </button>
    {/each}
  </div>

  {#if tab === "open"}
    {#if openProject && resolve.state === "connected"}
      <div class="flex items-center gap-3 rounded-lg border p-3">
        <div class="min-w-0 flex-1">
          <p class="truncate text-sm font-medium">{openProject}</p>
          <p class="text-xs text-muted-foreground">
            {resolve.database ? `${resolve.database.name} · ${resolve.database.type}` : "Open in Resolve now"}
          </p>
        </div>
        <button
          type="button"
          class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90"
          onclick={() => add({ kind: "open" })}
        >
          Add
        </button>
      </div>
    {:else}
      <p class="text-sm text-muted-foreground">Open a project in Resolve to add it here, or pick one from Resolve's database.</p>
    {/if}
  {:else if tab === "library"}
    {#if library === null}
      <p class="py-6 text-center text-xs text-muted-foreground">Asking Resolve for its projects…</p>
    {:else if "error" in library}
      <p class="text-sm text-destructive">{library.error}</p>
    {:else}
      <input class={field} placeholder="Filter projects" bind:value={filter} />
      <div class="-mx-1 max-h-72 overflow-y-auto">
        {#each projects as project (`${project.folder.join("/")}/${project.name}`)}
          <button
            type="button"
            class="flex w-full flex-col rounded-md px-2 py-1.5 text-left transition-colors hover:bg-accent"
            onclick={() => add({ kind: "library", name: project.name, folder: project.folder, database: library.database })}
          >
            <span class="truncate text-sm">{project.name}</span>
            {#if project.folder.length}
              <span class="truncate text-2xs text-muted-foreground">{project.folder.join(" / ")}</span>
            {/if}
          </button>
        {:else}
          <p class="py-4 text-center text-xs text-muted-foreground">No projects match.</p>
        {/each}
      </div>
      <p class="text-xs text-muted-foreground">
        From {library.database ? `"${library.database.name}"` : "the database Resolve has open"}.
        {#if otherDatabases.length}
          Resolve also has {otherDatabases.map((db) => `"${db.name}"`).join(", ")}: switch to it in Resolve's Project Manager to see its
          projects here.
        {/if}
      </p>
    {/if}
  {:else if tab === "cloud"}
    <label class="flex flex-col gap-1 text-xs text-muted-foreground">
      Project name, exactly as in Resolve
      <input class={field} bind:value={cloudName} />
    </label>
    <label class="flex flex-col gap-1 text-xs text-muted-foreground">
      Media folder on this computer
      <input class={field} placeholder="D:\Cloud media\Project" bind:value={mediaPath} />
    </label>
    <label class="flex flex-col gap-1 text-xs text-muted-foreground">
      Sync
      <select class={field} bind:value={syncMode}>
        {#each SYNC_MODES as mode (mode.value)}
          <option value={mode.value}>{mode.label}</option>
        {/each}
      </select>
    </label>
    <p class="text-xs text-muted-foreground">
      Resolve can't list {LIBRARY_LABEL.cloud} projects to Linker-Edit. If the project is open in Resolve now, add it from the first tab
      instead.
    </p>
    <button
      type="button"
      class="h-8 self-end rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40"
      disabled={!cloudName.trim() || !mediaPath.trim()}
      onclick={() => add({ kind: "cloud", name: cloudName.trim(), mediaPath: mediaPath.trim(), syncMode })}
    >
      Add
    </button>
  {:else}
    <label class="flex flex-col gap-1 text-xs text-muted-foreground">
      Project folder
      <input class={field} placeholder="C:\Users\you\LinkerEdit\projects\Promo" bind:value={folder} />
    </label>
    <p class="text-xs text-muted-foreground">
      A Linker-Edit project folder with its linker-edit.json, such as a teammate's edit history you cloned. It stays where it is.
    </p>
    <button
      type="button"
      class="h-8 self-end rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40"
      disabled={!folder.trim()}
      onclick={() => add({ kind: "folder", dir: folder.trim() })}
    >
      Add
    </button>
  {/if}
</Dialog>
