<script lang="ts">
  import { untrack } from "svelte";
  import ChevronLeft from "@lucide/svelte/icons/chevron-left";
  import Cloud from "@lucide/svelte/icons/cloud";
  import FilePlus from "@lucide/svelte/icons/file-plus-2";
  import FolderInput from "@lucide/svelte/icons/folder-input";
  import HardDrive from "@lucide/svelte/icons/hard-drive";
  import Monitor from "@lucide/svelte/icons/monitor";
  import MonitorUp from "@lucide/svelte/icons/monitor-up";
  import type { ProjectSource } from "#shared/protocol.ts";
  import { trimSeparators } from "#lib/browsePath.ts";
  import { LIBRARY_LABEL } from "#lib/labels.ts";
  import type { MachineConnection } from "#lib/machines.svelte.ts";
  import Dialog from "./Dialog.svelte";
  import FolderPicker from "./FolderPicker.svelte";

  let {
    machines,
    onAdd,
    onClose,
  }: {
    /** Every machine, this computer first: the project is added on the one chosen, and its threads run there. */
    machines: MachineConnection[];
    onAdd: (machineId: string, source: ProjectSource) => void;
    onClose: () => void;
  } = $props();

  // Like T3 Code, a first step chooses the machine when there's more than one.
  const multi = untrack(() => machines.length > 1);
  let machineId = $state<string | null>(untrack(() => (multi ? null : (machines[0]?.id ?? null))));
  const machine = $derived(machines.find((entry) => entry.id === machineId) ?? null);
  const server = $derived(machine?.server ?? null);
  const resolve = $derived(server?.state?.resolve ?? { state: "checking" as const });
  /** Null while it loads. */
  const library = $derived(server?.library ?? null);
  const browseFolders = (path: string) => server?.browseFolders(path) ?? Promise.resolve({ error: "Not connected." });
  const STATUS = { connecting: "Connecting…", offline: "Offline" };

  const TABS = [
    { value: "new", label: "New", Icon: FilePlus },
    { value: "open", label: "Open in Resolve", Icon: MonitorUp },
    { value: "library", label: "Database", Icon: HardDrive },
    { value: "cloud", label: "Cloud", Icon: Cloud },
    { value: "folder", label: "Folder", Icon: FolderInput },
  ] as const;

  const openProject = $derived(resolve.state === "connected" ? resolve.project : null);
  // Starts where the likely project is; the user picks the tab after that.
  function tabFor(id: string | null): (typeof TABS)[number]["value"] {
    const open = machines.find((entry) => entry.id === id)?.server.state?.resolve;
    return open?.state === "connected" && open.project ? "open" : "library";
  }
  let tab = $state(untrack(() => tabFor(machineId)));
  function choose(id: string) {
    machineId = id;
    tab = tabFor(id);
  }
  let filter = $state("");
  let newName = $state("");
  let cloudName = $state("");
  let folder = $state("");
  let folderStatus = $state<{ exists: boolean; isProject: boolean } | null>(null);
  const folderHint = $derived.by(() => {
    if (!folder.trim()) return "Pick the folder of a Linker-Edit project.";
    if (!folderStatus) return "Checking the folder…";
    if (!folderStatus.exists) return "There's no such folder.";
    if (!folderStatus.isProject) return "Go into a folder marked as a Linker-Edit project to add it.";
    return null;
  });

  function addFolder() {
    if (folderStatus?.isProject) add({ kind: "folder", dir: trimSeparators(folder) });
  }

  $effect(() => {
    // Asks the machine's Resolve for its projects each time the tab opens, so the list is current.
    if (tab !== "library" || !server) return;
    untrack(() => {
      server.library = null;
      server.send({ type: "load_library" });
    });
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
    if (!machineId) return;
    onAdd(machineId, source);
    onClose();
  }

  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm text-foreground outline-none focus:border-primary";
</script>

<Dialog title="Add project" {onClose}>
  {#if !machine}
    <p class="text-sm text-muted-foreground">Which computer is the project on? Its threads run there.</p>
    <ul class="flex flex-col gap-1" aria-label="Computers">
      {#each machines as entry (entry.id)}
        <li>
          <button
            type="button"
            class="flex h-11 w-full items-center gap-3 rounded-lg border px-3 text-left transition-colors hover:bg-accent disabled:opacity-50 disabled:hover:bg-transparent"
            disabled={entry.status !== "connected"}
            onclick={() => choose(entry.id)}
          >
            <Monitor class="size-4 shrink-0 text-muted-foreground" />
            <span class="min-w-0 flex-1 truncate text-sm">{entry.name}</span>
            {#if entry.status !== "connected"}
              <span class="shrink-0 text-2xs text-muted-foreground">{STATUS[entry.status]}</span>
            {:else if entry.machine.via === "this"}
              <span class="shrink-0 text-2xs text-muted-foreground">This computer</span>
            {/if}
          </button>
        </li>
      {:else}
        <li class="text-xs text-muted-foreground">No computer yet: Linker-Edit needs to run on the one the project is on.</li>
      {/each}
    </ul>
  {:else}
    {#if multi}
      <button
        type="button"
        class="-mt-2 flex items-center gap-1 self-start text-xs text-muted-foreground transition-colors hover:text-foreground"
        title="Choose another computer"
        onclick={() => (machineId = null)}
      >
        <ChevronLeft class="size-3.5" />
        <Monitor class="size-3.5" />
        On {machine.name}
      </button>
    {/if}
    <div class="flex gap-1 rounded-lg bg-accent p-1" role="tablist">
      {#each TABS as item (item.value)}
        <button
          type="button"
          role="tab"
          aria-selected={tab === item.value}
          class="flex h-7 flex-auto items-center justify-center gap-1.5 rounded-md text-xs transition-colors {tab === item.value
            ? 'bg-card text-foreground shadow-sm'
            : 'text-muted-foreground hover:text-foreground'}"
          onclick={() => (tab = item.value)}
        >
          <item.Icon class="size-3.5 max-sm:hidden" />
          {item.label}
        </button>
      {/each}
    </div>

    {#if tab === "new"}
      <form
        class="flex flex-col gap-3"
        onsubmit={(event) => {
          event.preventDefault();
          if (newName.trim()) add({ kind: "new", name: newName.trim() });
        }}
      >
        <label class="flex flex-col gap-1 text-xs text-muted-foreground">
          Project name
          <input class={field} maxlength="120" bind:value={newName} />
        </label>
        <div class="flex items-center gap-3">
          <p class="flex-1 text-xs text-muted-foreground">
            Makes an empty project in {resolve.state === "connected" && resolve.database ? `"${resolve.database.name}"` : "the database Resolve has open"}
            and opens it.{#if openProject}&nbsp;Resolve saves and closes "{openProject}" first.{/if}
          </p>
          <button
            type="submit"
            class="h-8 shrink-0 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40"
            disabled={!newName.trim()}
          >
            Make
          </button>
        </div>
      </form>
    {:else if tab === "open"}
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
      <p class="text-xs text-muted-foreground">
        You open {LIBRARY_LABEL.cloud} projects in Resolve's Project Manager yourself: {LIBRARY_LABEL.cloud}'s terms don't allow scripts
        to use it, so Linker-Edit never opens, creates or uploads cloud projects. If the project is open in Resolve now, add it from the
        first tab instead.
      </p>
      <button
        type="button"
        class="h-8 self-end rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40"
        disabled={!cloudName.trim()}
        onclick={() => add({ kind: "cloud", name: cloudName.trim() })}
      >
        Add
      </button>
    {:else}
      <FolderPicker
        bind:value={folder}
        bind:status={folderStatus}
        browse={browseFolders}
        native={machine.host.local}
        autofocus
        label="Project folder"
        onSubmit={addFolder}
      />
      <div class="flex items-center gap-3">
        <p class="flex-1 text-xs text-muted-foreground">
          {folderHint ?? "A Linker-Edit project folder, such as a teammate's edit history you cloned. It stays where it is."}
        </p>
        <button
          type="button"
          class="h-8 shrink-0 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-40"
          disabled={!folderStatus?.isProject}
          onclick={addFolder}
        >
          Add
        </button>
      </div>
    {/if}
  {/if}
</Dialog>
