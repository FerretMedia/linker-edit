<script lang="ts">
  import ArrowLeft from "@lucide/svelte/icons/arrow-left";
  import FolderOpen from "@lucide/svelte/icons/folder-open";
  import Globe from "@lucide/svelte/icons/globe";
  import Pause from "@lucide/svelte/icons/pause";
  import Pencil from "@lucide/svelte/icons/pencil";
  import Play from "@lucide/svelte/icons/play";
  import Plus from "@lucide/svelte/icons/plus";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import Search from "@lucide/svelte/icons/search";
  import Star from "@lucide/svelte/icons/star";
  import Trash2 from "@lucide/svelte/icons/trash";
  import { onDestroy } from "svelte";
  import {
    LIBRARY_KINDS,
    type AppMeta,
    type ClientMessage,
    type FolderListing,
    type LibraryFile,
    type LibraryFolder,
    type LibraryFolderStatus,
    type LibraryKind,
    type LibraryQuery,
    type LibrarySort,
    type LibraryView,
  } from "#shared/protocol.ts";
  import { fileUrl, formatDuration, KIND_LABEL, parseTags, playable, scopeOf, showable, type LibraryTab } from "#lib/library.ts";
  import { KIND_ICON } from "#lib/libraryIcons.ts";
  import LibraryFolderDialog from "./LibraryFolderDialog.svelte";
  import SettingsGroup from "./SettingsGroup.svelte";

  let {
    app,
    tab,
    view,
    connected,
    onTab,
    send,
    browseFolders,
    sortFolder,
    onClose,
  }: {
    app: AppMeta;
    tab: LibraryTab;
    /** What the server sent for this page; it may still be of the tab before. */
    view: LibraryView | null;
    connected: boolean;
    onTab: (tab: LibraryTab) => void;
    send: (message: ClientMessage) => void;
    browseFolders: (path: string) => Promise<FolderListing | { error: string }>;
    sortFolder: (path: string) => Promise<LibrarySort | { error: string }>;
    onClose: () => void;
  } = $props();

  const project = $derived(app.projects.find((candidate) => candidate.id === app.projectId) ?? null);
  const scope = $derived(scopeOf(tab, app.projectId));
  const current = $derived(view && scope && JSON.stringify(view.scope) === JSON.stringify(scope) ? view : null);
  const where = $derived(tab === "universal" ? "the universal library, for every project" : `the library of "${project?.name ?? "this project"}"`);

  let search = $state("");
  let kind = $state<LibraryKind | "">("");
  let starred = $state(false);
  // Typing waits a moment before asking the server again.
  let typed = $state("");
  $effect(() => {
    const value = search;
    const timer = setTimeout(() => (typed = value), 150);
    return () => clearTimeout(timer);
  });
  const query = $derived<LibraryQuery>({ ...(typed.trim() ? { query: typed.trim() } : {}), ...(kind ? { kind } : {}), ...(starred ? { starred } : {}) });

  $effect(() => {
    // Asks again after a reconnect, a switch of tab or project, or a new search; the server sends every change after.
    if (connected && scope) send({ type: "library_open", scope, query });
  });
  onDestroy(() => send({ type: "library_close" }));

  let dialog = $state<{ folder: LibraryFolder | null } | null>(null);

  function saveFolder(fields: Omit<LibraryFolder, "id">) {
    if (!scope) return;
    if (dialog?.folder) send({ type: "library_update_folder", scope, folderId: dialog.folder.id, changes: fields });
    else send({ type: "library_add_folder", scope, folder: fields });
  }

  function saveFolders(folders: Omit<LibraryFolder, "id">[]) {
    if (scope && folders.length) send({ type: "library_add_folders", scope, folders });
  }

  function removeFolder(folder: LibraryFolderStatus) {
    if (!scope) return;
    if (confirm(`Remove "${folder.label}" from the library? The folder and its files stay where they are.`)) {
      send({ type: "library_remove_folder", scope, folderId: folder.id });
    }
  }

  // One file plays at a time, in place.
  let audio: HTMLAudioElement | undefined = $state();
  let playing = $state<string | null>(null);
  const keyOf = (file: LibraryFile) => `${file.folderId}/${file.rel}`;
  function toggle(file: LibraryFile) {
    if (!audio) return;
    if (playing === keyOf(file)) {
      audio.pause();
      return;
    }
    audio.src = fileUrl(file.folderId, file.rel);
    playing = keyOf(file);
    audio.play().catch(() => (playing = null));
  }
  onDestroy(() => audio?.pause());

  let open = $state<string | null>(null);
  let tagsText = $state("");
  let noteText = $state("");
  function edit(file: LibraryFile) {
    if (open === keyOf(file)) {
      open = null;
      return;
    }
    open = keyOf(file);
    tagsText = file.tags.join(", ");
    noteText = file.note;
  }
  function saveFile(file: LibraryFile) {
    send({ type: "library_update_file", folderId: file.folderId, rel: file.rel, tags: parseTags(tagsText), note: noteText });
    open = null;
  }

  const folderOf = (file: LibraryFile) => current?.folders.find((folder) => folder.id === file.folderId);
  const dirOf = (file: LibraryFile) => file.rel.split("/").slice(0, -1).join("/");
  const ago = (iso: string) => new Date(iso).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });

  const button = "flex h-8 items-center gap-1.5 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
  const iconButton =
    "flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground";
</script>

<audio bind:this={audio} onpause={() => (playing = null)} onended={() => (playing = null)}></audio>

<div class="title-bar-row flex h-full flex-col md:flex-row">
  <nav class="flex shrink-0 flex-col border-b bg-sidebar md:w-60 md:border-r md:border-b-0" aria-label="Library">
    <div class="title-bar title-bar-start flex h-13 shrink-0 items-center px-2">
      <button
        type="button"
        class="flex h-8 items-center gap-2 rounded-lg px-2.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="Back (Esc)"
        onclick={onClose}
      >
        <ArrowLeft class="size-4" />
        Back
      </button>
    </div>
    <ul class="flex gap-1 overflow-x-auto px-2 pb-2 md:flex-col md:overflow-visible">
      {#each [{ id: "universal", label: "Universal", Icon: Globe }, { id: "project", label: "This project", Icon: FolderOpen }] as const as entry (entry.id)}
        <li class="shrink-0">
          <button
            type="button"
            class="flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm whitespace-nowrap transition-colors disabled:opacity-50 {entry.id === tab
              ? 'bg-sidebar-row-active font-medium text-foreground shadow-[0_0_0_1px_var(--border)]'
              : 'text-muted-foreground hover:bg-accent hover:text-foreground'}"
            aria-current={entry.id === tab ? "page" : undefined}
            disabled={entry.id === "project" && !project}
            title={entry.id === "project" ? (project?.name ?? "Add a project first") : "Every project"}
            onclick={() => onTab(entry.id)}
          >
            <entry.Icon class="size-4 shrink-0" />
            {entry.label}
          </button>
        </li>
      {/each}
    </ul>
    <p class="mt-auto hidden px-4 py-3 text-2xs text-muted-foreground md:block">
      Files reused across your edits. Agents look here first for music, sound effects and graphics.
    </p>
  </nav>

  <main class="flex min-h-0 min-w-0 flex-1 flex-col">
    <header class="title-bar flex h-13 shrink-0 items-center gap-2 border-b px-5 md:px-8">
      <h1 class="min-w-0 truncate text-sm font-medium">
        {tab === "universal" ? "Universal library" : `Library of ${project?.name ?? "this project"}`}
      </h1>
      {#if scope}
        <button type="button" class="{button} ml-auto" onclick={() => (dialog = { folder: null })}>
          <Plus class="size-3.5" />
          Add folder
        </button>
      {/if}
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto">
      <div class="mx-auto flex w-full max-w-3xl flex-col gap-8 px-4 py-6 md:px-8 md:py-8">
        {#if !scope}
          <p class="text-sm text-muted-foreground">Add a project first. Its own library is for files only that project uses.</p>
        {:else if !current}
          <p class="text-sm text-muted-foreground">{connected ? "Loading…" : "Connecting to the Linker-Edit server…"}</p>
        {:else}
          <SettingsGroup
            title="Folders"
            description={tab === "universal"
              ? "Every project can use these. Linker-Edit never copies, moves or deletes your files."
              : "Only this project uses these, before the universal library. Kept on this computer only."}
          >
            {#each current.folders as folder (folder.id)}
              {@const Icon = KIND_ICON[folder.kind]}
              <div class="flex flex-col gap-3 p-4 sm:flex-row sm:items-start sm:gap-4">
                <Icon class="mt-0.5 hidden size-4 shrink-0 text-muted-foreground sm:block" />
                <div class="min-w-0 flex-1">
                  <p class="text-sm">{folder.label}</p>
                  <p class="mt-0.5 text-xs text-muted-foreground">
                    {KIND_LABEL[folder.kind]} ·
                    {#if folder.scanning}
                      Reading {folder.scanning.done} of {folder.scanning.total || "…"}
                    {:else}
                      {folder.files} {folder.files === 1 ? "file" : "files"}{folder.scannedAt ? `, read ${ago(folder.scannedAt)}` : ""}
                    {/if}
                  </p>
                  <code class="mt-1 block truncate font-mono text-2xs text-muted-foreground" title={folder.path}>{folder.path}</code>
                  {#if folder.note}
                    <p class="mt-1 text-xs whitespace-pre-wrap">{folder.note}</p>
                  {/if}
                  {#if folder.problem}
                    <p class="mt-1 text-xs text-destructive">{folder.problem}</p>
                  {/if}
                  {#if folder.scanning && folder.scanning.total}
                    <div class="mt-2 h-1 overflow-hidden rounded-full bg-border">
                      <div class="h-full rounded-full bg-primary transition-[width]" style:width="{(folder.scanning.done / folder.scanning.total) * 100}%"></div>
                    </div>
                  {/if}
                </div>
                <div class="flex shrink-0 items-center gap-1">
                  <button
                    type="button"
                    class={iconButton}
                    aria-label="Rescan {folder.label}"
                    title="Rescan: reads new and changed files"
                    disabled={!!folder.scanning}
                    onclick={() => send({ type: "library_rescan", folderId: folder.id })}
                  >
                    <RefreshCw class="size-3.5 {folder.scanning ? 'animate-spin' : ''}" />
                  </button>
                  <button type="button" class={iconButton} aria-label="Edit {folder.label}" title="Edit" onclick={() => (dialog = { folder })}>
                    <Pencil class="size-3.5" />
                  </button>
                  <button
                    type="button"
                    class="{iconButton} hover:text-destructive"
                    aria-label="Remove {folder.label}"
                    title="Remove from the library. The folder stays."
                    onclick={() => removeFolder(folder)}
                  >
                    <Trash2 class="size-3.5" />
                  </button>
                </div>
              </div>
            {:else}
              <div class="flex flex-col items-start gap-3 p-4">
                <p class="text-sm text-muted-foreground">No folders yet. Add the folders where you keep music, sound effects, logos or LUTs.</p>
                <button type="button" class={button} onclick={() => (dialog = { folder: null })}>
                  <Plus class="size-3.5" />
                  Add folder
                </button>
              </div>
            {/each}
          </SettingsGroup>

          {#if current.folders.length}
            <section class="flex flex-col gap-2">
              <div class="flex flex-wrap items-center gap-2">
                <label class="flex h-8 min-w-48 flex-1 items-center gap-2 rounded-md border px-2.5 focus-within:border-primary">
                  <Search class="size-3.5 shrink-0 text-muted-foreground" />
                  <input class="min-w-0 flex-1 bg-transparent text-sm outline-none" placeholder="Search names, tags and notes" aria-label="Search the library" bind:value={search} />
                </label>
                <select class="h-8 rounded-md border bg-transparent px-2 text-xs outline-none focus:border-primary" aria-label="Kind" bind:value={kind}>
                  <option value="">All kinds</option>
                  {#each LIBRARY_KINDS as option (option)}
                    <option value={option}>{KIND_LABEL[option]}</option>
                  {/each}
                </select>
                <button
                  type="button"
                  class="{button} {starred ? 'bg-accent text-foreground' : 'text-muted-foreground'}"
                  aria-pressed={starred}
                  onclick={() => (starred = !starred)}
                >
                  <Star class="size-3.5 {starred ? 'fill-current text-warning' : ''}" />
                  Starred
                </button>
              </div>

              <div class="divide-y rounded-xl border bg-card" role="list" aria-label="Files">
                {#each current.files as file (keyOf(file))}
                  {@const key = keyOf(file)}
                  <div class="flex flex-col" role="listitem">
                    <div class="flex items-center gap-2 px-3 py-2">
                      {#if showable(file.name)}
                        <img src={fileUrl(file.folderId, file.rel)} alt="" loading="lazy" class="size-7 shrink-0 rounded object-cover" />
                      {:else}
                        <button
                          type="button"
                          class="{iconButton} disabled:opacity-30"
                          aria-label="{playing === key ? 'Pause' : 'Play'} {file.name}"
                          disabled={!playable(file.name)}
                          onclick={() => toggle(file)}
                        >
                          {#if playing === key}<Pause class="size-3.5" />{:else}<Play class="size-3.5" />{/if}
                        </button>
                      {/if}
                      <button type="button" class="min-w-0 flex-1 text-left" title="Tags and note" onclick={() => edit(file)}>
                        <p class="truncate text-sm">{file.name}</p>
                        <p class="truncate text-2xs text-muted-foreground">
                          {[
                            current.folders.length > 1 ? folderOf(file)?.label : null,
                            dirOf(file) || null,
                            file.duration !== null ? formatDuration(file.duration) : null,
                            file.loudness !== null ? `${Math.round(file.loudness)} LUFS` : null,
                            file.channels === 1 ? "mono" : file.channels === 2 ? "stereo" : file.channels ? `${file.channels} ch` : null,
                            file.usedIn.length ? `used ${file.usedIn.length}×` : null,
                          ]
                            .filter(Boolean)
                            .join(" · ")}
                        </p>
                      </button>
                      {#each file.tags.slice(0, 3) as tag (tag)}
                        <span class="hidden shrink-0 rounded bg-accent px-1.5 text-2xs text-muted-foreground sm:inline">{tag}</span>
                      {/each}
                      <button
                        type="button"
                        class={iconButton}
                        aria-label="{file.star ? 'Unstar' : 'Star'} {file.name}"
                        aria-pressed={file.star}
                        onclick={() => send({ type: "library_update_file", folderId: file.folderId, rel: file.rel, star: !file.star })}
                      >
                        <Star class="size-3.5 {file.star ? 'fill-current text-warning' : ''}" />
                      </button>
                    </div>
                    {#if open === key}
                      <div class="flex flex-col gap-2 border-t bg-muted/40 px-3 py-3">
                        <label class="flex flex-col gap-1 text-xs text-muted-foreground">
                          Tags, with commas
                          <input
                            class="h-8 rounded-md border bg-card px-2.5 text-sm text-foreground outline-none focus:border-primary"
                            placeholder="upbeat, intro"
                            bind:value={tagsText}
                            onkeydown={(event) => event.key === "Enter" && saveFile(file)}
                          />
                        </label>
                        <label class="flex flex-col gap-1 text-xs text-muted-foreground">
                          Note
                          <textarea
                            class="min-h-14 rounded-md border bg-card px-2.5 py-1.5 text-sm text-foreground outline-none focus:border-primary"
                            placeholder="Good under talking heads."
                            bind:value={noteText}
                          ></textarea>
                        </label>
                        {#if file.usedIn.length}
                          <p class="text-xs text-muted-foreground">
                            Used in {file.usedIn.map((use) => (use.timeline ? `${use.project} · ${use.timeline}` : use.project)).join(", ")}
                          </p>
                        {/if}
                        <p class="truncate font-mono text-2xs text-muted-foreground" title={file.path}>{file.path}</p>
                        <div class="flex justify-end gap-2">
                          <button type="button" class="h-8 rounded-md px-3 text-xs transition-colors hover:bg-accent" onclick={() => (open = null)}>Cancel</button>
                          <button
                            type="button"
                            class="h-8 rounded-md bg-primary px-3 text-xs text-primary-foreground transition-opacity hover:opacity-90"
                            onclick={() => saveFile(file)}
                          >
                            Save
                          </button>
                        </div>
                      </div>
                    {/if}
                  </div>
                {:else}
                  <p class="p-4 text-center text-sm text-muted-foreground">
                    {current.folders.some((folder) => folder.scanning) ? "Reading the folders…" : typed || kind || starred ? "Nothing matches." : "No files in these folders."}
                  </p>
                {/each}
              </div>
              {#if current.total > current.files.length}
                <p class="px-1 text-xs text-muted-foreground">Showing {current.files.length} of {current.total}. Search to narrow it down.</p>
              {/if}
            </section>
          {/if}
        {/if}
      </div>
    </div>
  </main>
</div>

{#if dialog}
  <LibraryFolderDialog
    folder={dialog.folder}
    {where}
    existing={current?.folders.map((folder) => folder.path) ?? []}
    {browseFolders}
    {sortFolder}
    onSave={saveFolder}
    onSaveMany={saveFolders}
    onClose={() => (dialog = null)}
  />
{/if}
