<script lang="ts">
  import { Plus } from "@lucide/svelte";
  import type { AppMeta, ClientMessage, CollectionKind, CollectionSummary } from "#shared/protocol.ts";
  import { COLLECTION_LABEL } from "#lib/labels.ts";
  import CollectionEditor from "./CollectionEditor.svelte";
  import CollectionIcon from "./CollectionIcon.svelte";

  // The workspaces & series section of the settings page: the list, making new ones, and the one picked.
  let { app, send }: { app: AppMeta; send: (message: ClientMessage) => void } = $props();

  const workspaces = $derived(app.collections.filter((collection) => collection.kind === "workspace"));
  const seriesIn = (parentId: string | null) =>
    app.collections.filter((collection) => collection.kind === "series" && collection.parentId === parentId);
  const loose = $derived(seriesIn(null));

  let selectedId = $state<string | null>(null);
  const selected = $derived(app.collections.find((collection) => collection.id === selectedId) ?? null);

  let creating = $state<{ kind: CollectionKind; name: string; parentId: string | null } | null>(null);
  // Picks the new one once the server has it.
  let known: Set<string> | null = null;
  $effect(() => {
    const { collections } = app;
    const added = known && collections.find((collection) => !known!.has(collection.id));
    if (added) {
      selectedId = added.id;
      known = null;
    }
  });

  function startCreating(kind: CollectionKind) {
    const parentId = kind === "series" ? selected?.kind === "workspace" ? selected.id : (selected?.parentId ?? null) : null;
    creating = { kind, name: "", parentId };
  }

  function create() {
    if (!creating?.name.trim()) return;
    known = new Set(app.collections.map((collection) => collection.id));
    send({ type: "create_collection", kind: creating.kind, name: creating.name, parentId: creating.parentId });
    creating = null;
  }

  function counts(collection: CollectionSummary): string {
    const series = collection.kind === "workspace" ? seriesIn(collection.id).length : 0;
    const projects = collection.projectIds.length;
    return [
      series ? `${series} ${series === 1 ? COLLECTION_LABEL.series.lower : COLLECTION_LABEL.series.lowerMany}` : "",
      `${projects} project${projects === 1 ? "" : "s"}`,
    ]
      .filter(Boolean)
      .join(" · ");
  }

  const projectNames = (collection: CollectionSummary) =>
    collection.projectIds.map((id) => app.projects.find((project) => project.id === id)?.name).filter((name): name is string => !!name);

  const button = "flex h-8 items-center gap-1.5 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent";
  const field = "h-8 min-w-0 flex-1 rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
</script>

{#snippet row(collection: CollectionSummary, nested: boolean)}
  <li>
    <button
      type="button"
      class="flex w-full items-center gap-3 px-4 py-2.5 text-left transition-colors hover:bg-accent {nested ? 'pl-11' : ''} {collection.id ===
      selectedId
        ? 'bg-accent'
        : ''}"
      aria-current={collection.id === selectedId ? "true" : undefined}
      onclick={() => (selectedId = collection.id === selectedId ? null : collection.id)}
    >
      <CollectionIcon icon={collection.icon} name={collection.name} />
      <span class="min-w-0 flex-1 truncate text-sm">{collection.name}</span>
      <span class="shrink-0 text-xs text-muted-foreground">{counts(collection)}</span>
    </button>
  </li>
{/snippet}

<section class="flex flex-col gap-2">
  <div class="flex flex-wrap items-end gap-2 px-1">
    <div class="min-w-0 flex-1">
      <p class="text-xs text-muted-foreground">
        A {COLLECTION_LABEL.workspace.lower} is a client, brand or channel. A {COLLECTION_LABEL.series.lower} holds its episodes. Each has
        its own rules for the agent.
      </p>
    </div>
    <button type="button" class={button} onclick={() => startCreating("workspace")}>
      <Plus class="size-3.5" />
      New {COLLECTION_LABEL.workspace.lower}
    </button>
    <button type="button" class={button} onclick={() => startCreating("series")}>
      <Plus class="size-3.5" />
      New {COLLECTION_LABEL.series.lower}
    </button>
  </div>

  <div class="overflow-hidden rounded-xl border bg-card">
    {#if app.collections.length}
      <ul class="divide-y">
        {#each workspaces as workspace (workspace.id)}
          {@render row(workspace, false)}
          {#each seriesIn(workspace.id) as series (series.id)}
            {@render row(series, true)}
          {/each}
        {/each}
        {#each loose as series (series.id)}
          {@render row(series, false)}
        {/each}
      </ul>
    {:else if !creating}
      <p class="p-4 text-xs text-muted-foreground">
        None yet. Make a {COLLECTION_LABEL.workspace.lower} for each client, brand or channel you edit for.
      </p>
    {/if}

    {#if creating}
      {@const kind = creating.kind}
      <form
        class="flex flex-wrap items-center gap-2 p-3 {app.collections.length ? 'border-t' : ''}"
        onsubmit={(event) => {
          event.preventDefault();
          create();
        }}
      >
        <input
          class={field}
          aria-label="New {COLLECTION_LABEL[kind].lower} name"
          placeholder="New {COLLECTION_LABEL[kind].lower} name"
          maxlength="80"
          {@attach (input) => input.focus()}
          bind:value={creating.name}
          onkeydown={(event) => event.key === "Escape" && (creating = null)}
        />
        {#if kind === "series"}
          <select class="{field} flex-none sm:w-44" aria-label="In {COLLECTION_LABEL.workspace.lower}" bind:value={creating.parentId}>
            <option value={null}>No {COLLECTION_LABEL.workspace.lower}</option>
            {#each workspaces as workspace (workspace.id)}
              <option value={workspace.id}>{workspace.name}</option>
            {/each}
          </select>
        {/if}
        <button type="button" class="h-8 rounded-md px-3 text-xs transition-colors hover:bg-accent" onclick={() => (creating = null)}>Cancel</button>
        <button
          type="submit"
          class="h-8 rounded-md bg-primary px-3 text-xs text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
          disabled={!creating.name.trim()}
        >
          Create
        </button>
      </form>
    {/if}
  </div>
</section>

{#if selected}
  {#key selected.id}
    <CollectionEditor
      collection={selected}
      {workspaces}
      projectNames={projectNames(selected)}
      seriesCount={selected.kind === "workspace" ? seriesIn(selected.id).length : 0}
      onUpdate={(changes) => send({ type: "update_collection", id: selected.id, ...changes })}
      onSetup={() => send({ type: "setup_collection", id: selected.id })}
      onRemoveLesson={(lessonId) => send({ type: "remove_lesson", id: selected.id, lessonId })}
      onRemoveExample={(name) => send({ type: "remove_example", id: selected.id, name })}
      onDelete={() => {
        send({ type: "delete_collection", id: selected.id });
        selectedId = null;
      }}
    />
  {/key}
{:else if app.collections.length}
  <p class="-mt-4 px-1 text-xs text-muted-foreground">Pick one to rename it, change its icon, write its rules or set it up with AI.</p>
{/if}

<p class="px-1 text-xs text-muted-foreground">
  A project can have its own notes for the agent too: a RULES.md in the project's folder. They come after the
  {COLLECTION_LABEL.workspace.lower}'s and {COLLECTION_LABEL.series.lower}' rules.
</p>
