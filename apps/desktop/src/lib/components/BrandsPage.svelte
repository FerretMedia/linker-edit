<script lang="ts">
  import { untrack } from "svelte";
  import ArrowLeft from "@lucide/svelte/icons/arrow-left";
  import Clapperboard from "@lucide/svelte/icons/clapperboard";
  import ExternalLink from "@lucide/svelte/icons/external-link";
  import Monitor from "@lucide/svelte/icons/monitor";
  import Plus from "@lucide/svelte/icons/plus";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import type { AppMeta, ClientMessage, CollectionKind, CollectionSummary } from "#shared/protocol.ts";
  import { parentOptions, roots, subtree } from "#lib/brands.ts";
  import type { Host } from "#lib/host.ts";
  import { provideHost } from "#lib/hostContext.ts";
  import { COLLECTION_LABEL, collectionLabel, NOT_IN_A_BRAND } from "#lib/labels.ts";
  import CollectionEditor from "./CollectionEditor.svelte";
  import CollectionIcon from "./CollectionIcon.svelte";
  import LinkerAccountRow from "./LinkerAccount.svelte";

  // Every brand (a Linker org) with its sub-brands, series and projects, and the brief of the one picked. Laid out
  // like the settings page: the brands on the left, the one open on the right.
  let {
    app,
    host,
    machines,
    machineId,
    onMachine,
    send,
    onClose,
  }: {
    /** The computer whose sub-brands and series these are: each keeps its own. */
    app: AppMeta;
    host: Host;
    machines: { id: string; name: string; status: "connecting" | "connected" | "offline" }[];
    machineId: string;
    onMachine: (machineId: string) => void;
    send: (message: ClientMessage) => void;
    onClose: () => void;
  } = $props();

  provideHost(() => host);

  // Linker is asked again each time the page opens, and for each computer picked.
  $effect(() => {
    void machineId;
    untrack(() => send({ type: "refresh_brands" }));
  });

  const LOOSE = "loose";
  const signedIn = $derived(app.linker.state === "signed_in");
  const brands = $derived(app.collections.filter((collection) => collection.kind === "brand"));
  /** Sub-brands and series from before brands, in none until they are moved. */
  const loose = $derived(roots(app.collections).filter((collection) => collection.kind !== "brand"));

  let picked = $state<string | null>(null);
  /** The brand open on the right, or LOOSE; the first brand the person is in until one is picked. */
  const view = $derived.by(() => {
    if (picked === LOOSE && loose.length) return LOOSE;
    if (picked && brands.some((brand) => brand.id === picked)) return picked;
    return (brands.find((brand) => brand.org?.member) ?? brands[0])?.id ?? (loose.length ? LOOSE : null);
  });
  const brand = $derived(brands.find((entry) => entry.id === view) ?? null);
  const rows = $derived((view === LOOSE ? loose : brand ? [brand] : []).flatMap((root) => subtree(app.collections, root)));

  let selectedId = $state<string | null>(null);
  const selected = $derived(rows.find((row) => row.collection.id === selectedId)?.collection ?? brand);

  const projectName = $derived(new Map(app.projects.map((project) => [project.id, project.name])));
  const projectNames = (collection: CollectionSummary) =>
    collection.projectIds.map((id) => projectName.get(id)).filter((name): name is string => !!name);

  function pick(id: string) {
    picked = id;
    selectedId = null;
    creating = null;
  }

  let creating = $state<{ kind: CollectionKind; name: string; parentId: string } | null>(null);
  /** Where a new one of this kind can go on the brand shown. */
  const places = (kind: CollectionKind) =>
    parentOptions(app.collections, kind).filter((option) => !option.disabled && rows.some((row) => row.collection.id === option.id));
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
    const options = places(kind);
    // In the one picked when it can hold it, else next to it, else in the brand shown.
    const parentId = [selected?.id, selected?.parentId, brand?.id].find((id) => options.some((option) => option.id === id)) ?? options[0]?.id;
    if (parentId) creating = { kind, name: "", parentId };
  }

  function create() {
    if (!creating?.name.trim()) return;
    known = new Set(app.collections.map((collection) => collection.id));
    send({ type: "create_collection", kind: creating.kind, name: creating.name, parentId: creating.parentId });
    creating = null;
  }

  /** Why a brand is greyed. */
  const closedNote = (entry: CollectionSummary) =>
    signedIn ? `You're not in ${entry.name} on Linker any more, so it takes nothing new.` : `Sign in with Linker to put things in ${entry.name}.`;

  const button = "flex h-8 items-center gap-1.5 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent";
  const field = "h-8 min-w-0 flex-1 rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
  const navItem = "flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm whitespace-nowrap transition-colors";
  const navActive = "bg-sidebar-row-active font-medium text-foreground shadow-[0_0_0_1px_var(--border)]";
  const navIdle = "text-muted-foreground hover:bg-accent hover:text-foreground";
</script>

<div class="title-bar-row flex h-full flex-col md:flex-row">
  <nav class="flex shrink-0 flex-col border-b bg-sidebar md:w-60 md:border-r md:border-b-0" aria-label="Brands">
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
    {#if machines.length > 1}
      <!-- Each computer keeps its own sub-brands and series, like its own settings. -->
      <ul class="flex gap-1 overflow-x-auto border-b px-2 pb-2 md:mb-2 md:flex-col md:overflow-visible" aria-label="Computers">
        {#each machines as entry (entry.id)}
          <li class="shrink-0">
            <button
              type="button"
              class="{navItem} disabled:opacity-50 {entry.id === machineId ? navActive : `${navIdle} disabled:hover:bg-transparent`}"
              aria-current={entry.id === machineId ? "true" : undefined}
              disabled={entry.status !== "connected"}
              title={entry.status === "connected" ? `Brands of Linker-Edit on ${entry.name}` : `${entry.name} is ${entry.status}`}
              onclick={() => onMachine(entry.id)}
            >
              <Monitor class="size-4 shrink-0" />
              <span class="min-w-0 flex-1 truncate">{entry.name}</span>
              {#if entry.status !== "connected"}
                <span class="shrink-0 text-2xs">{entry.status}</span>
              {/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    <ul class="flex gap-1 overflow-x-auto px-2 pb-2 md:flex-col md:overflow-visible">
      {#each brands as entry (entry.id)}
        <li class="shrink-0">
          <button
            type="button"
            class="{navItem} {entry.id === view ? navActive : navIdle} {entry.org?.member ? '' : 'opacity-50'}"
            aria-current={entry.id === view ? "page" : undefined}
            title={entry.org?.member ? undefined : closedNote(entry)}
            onclick={() => pick(entry.id)}
          >
            <CollectionIcon icon={entry.icon} name={entry.name} picture={entry.org?.imageUrl} size="sm" />
            <span class="min-w-0 flex-1 truncate">{entry.name}</span>
          </button>
        </li>
      {/each}
      {#if loose.length}
        <li class="shrink-0 md:mt-2 md:border-t md:pt-2">
          <button
            type="button"
            class="{navItem} {view === LOOSE ? navActive : navIdle}"
            aria-current={view === LOOSE ? "page" : undefined}
            onclick={() => pick(LOOSE)}
          >
            <span class="min-w-0 flex-1 truncate">{NOT_IN_A_BRAND}</span>
            <span class="shrink-0 text-2xs">{loose.length}</span>
          </button>
        </li>
      {/if}
    </ul>
    <p class="mt-auto hidden px-4 py-3 text-2xs text-muted-foreground md:block">
      {COLLECTION_LABEL.subbrand.many} and {COLLECTION_LABEL.series.lowerMany} of Linker-Edit on <span class="text-foreground">{app.computer}</span>
    </p>
  </nav>

  <main class="flex min-h-0 min-w-0 flex-1 flex-col">
    <header class="title-bar flex h-13 shrink-0 items-center gap-2 border-b px-5 md:px-8">
      <h1 class="min-w-0 flex-1 truncate text-sm font-medium">{brand?.name ?? (view === LOOSE ? NOT_IN_A_BRAND : COLLECTION_LABEL.brand.many)}</h1>
      {#if signedIn}
        <button
          type="button"
          class="flex size-8 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
          aria-label="Refresh brands from Linker"
          title="Refresh brands from Linker"
          onclick={() => send({ type: "refresh_brands" })}
        >
          <RefreshCw class="size-4" />
        </button>
      {/if}
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto">
      <div class="mx-auto flex w-full max-w-2xl flex-col gap-8 px-4 py-6 md:px-8 md:py-8">
        {#if !signedIn}
          <section class="flex flex-col gap-2 rounded-xl border bg-card p-4">
            <h2 class="text-sm font-medium">Your brands are your Linker orgs</h2>
            <p class="text-xs text-muted-foreground">
              Sign in with Linker to see every org you're in as a brand, and to make {COLLECTION_LABEL.subbrand.lowerMany} and
              {COLLECTION_LABEL.series.lowerMany} in it.
            </p>
            <div class="-mx-4">
              <LinkerAccountRow
                linker={app.linker}
                computer={app.computer}
                onCancel={() => send({ type: "linker_cancel_sign_in" })}
                onSignOut={() => send({ type: "linker_sign_out" })}
              />
            </div>
          </section>
        {:else if !brands.length}
          <section class="flex flex-col gap-2 rounded-xl border bg-card p-4">
            <h2 class="text-sm font-medium">No brands yet</h2>
            <p class="text-xs text-muted-foreground">Each org you're in on Linker is a brand here. Make one in Linker, then refresh.</p>
            {#if app.linker.state === "signed_in"}
              <a class="flex items-center gap-1.5 self-start text-xs text-primary hover:underline" href={app.linker.appUrl} target="_blank" rel="noreferrer">
                <ExternalLink class="size-3.5" />
                Open Linker
              </a>
            {/if}
          </section>
        {/if}

        {#if rows.length}
          <section class="flex flex-col gap-2">
            <div class="flex flex-wrap items-end gap-2 px-1">
              <p class="min-w-0 flex-1 text-xs text-muted-foreground max-sm:basis-full">
                {#if brand && !brand.org?.member}
                  {closedNote(brand)}
                {:else if view === LOOSE}
                  These are from before brands. Move each into a {COLLECTION_LABEL.brand.lower}.
                {:else}
                  A {COLLECTION_LABEL.subbrand.lower} is a part of the brand, such as a channel. A {COLLECTION_LABEL.series.lower} holds its
                  episodes. Each has its own rules for the agent.
                {/if}
              </p>
              {#if places("subbrand").length}
                <button type="button" class={button} onclick={() => startCreating("subbrand")}>
                  <Plus class="size-3.5" />
                  New {COLLECTION_LABEL.subbrand.lower}
                </button>
              {/if}
              {#if places("series").length}
                <button type="button" class={button} onclick={() => startCreating("series")}>
                  <Plus class="size-3.5" />
                  New {COLLECTION_LABEL.series.lower}
                </button>
              {/if}
            </div>

            <div class="overflow-hidden rounded-xl border bg-card {brand && !brand.org?.member ? 'opacity-60' : ''}">
              <ul class="divide-y" aria-label="{brand?.name ?? NOT_IN_A_BRAND}: {COLLECTION_LABEL.subbrand.lowerMany}, {COLLECTION_LABEL.series.lowerMany} and projects">
                {#each rows as { collection, depth } (collection.id)}
                  {@const names = projectNames(collection)}
                  <li>
                    <div class="flex items-center transition-colors hover:bg-accent {collection.id === selected?.id ? 'bg-accent' : ''}">
                      <button
                        type="button"
                        class="flex min-w-0 flex-1 items-center gap-3 py-2.5 pr-4 text-left"
                        style:padding-left="{1 + depth * 1.75}rem"
                        aria-current={collection.id === selected?.id ? "true" : undefined}
                        onclick={() => (selectedId = collection.id)}
                      >
                        <CollectionIcon icon={collection.icon} name={collection.name} picture={collection.org?.imageUrl} />
                        <span class="min-w-0 flex-1 truncate text-sm">{collection.name}</span>
                        <span class="shrink-0 text-xs text-muted-foreground">
                          {collectionLabel(collection.kind).one}{names.length ? ` · ${names.length} project${names.length === 1 ? "" : "s"}` : ""}
                        </span>
                      </button>
                      {#if collection.kind !== "brand" && !collection.parentId}
                        <select
                          class="mr-3 h-7 w-28 shrink-0 rounded-md border bg-card px-2 text-xs"
                          aria-label="Move {collection.name} to"
                          value=""
                          onchange={(event) => event.currentTarget.value && send({ type: "update_collection", id: collection.id, parentId: event.currentTarget.value })}
                        >
                          <option value="" disabled>Move to…</option>
                          {#each parentOptions(app.collections, collection.kind) as option (option.id)}
                            <option value={option.id} disabled={option.disabled}>{option.label}</option>
                          {/each}
                        </select>
                      {/if}
                    </div>
                    {#if names.length}
                      <ul aria-label="Projects in {collection.name}">
                        {#each names as name, index (index)}
                          <li class="flex items-center gap-2 py-1.5 pr-4 text-xs text-muted-foreground" style:padding-left="{1.4 + (depth + 1) * 1.75}rem">
                            <Clapperboard class="size-3.5 shrink-0" />
                            <span class="min-w-0 truncate">{name}</span>
                          </li>
                        {/each}
                      </ul>
                    {/if}
                  </li>
                {/each}
              </ul>

              {#if creating}
                {@const kind = creating.kind}
                <form
                  class="flex flex-wrap items-center gap-2 border-t p-3"
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
                  <select class="{field} flex-none sm:w-48" aria-label="New {COLLECTION_LABEL[kind].lower} in" bind:value={creating.parentId}>
                    {#each places(kind) as option (option.id)}
                      <option value={option.id}>{option.label}</option>
                    {/each}
                  </select>
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
        {/if}

        {#if selected}
          {#key selected.id}
            <CollectionEditor
              collection={selected}
              parents={parentOptions(app.collections, selected.kind)}
              projectNames={projectNames(selected)}
              childCount={app.collections.filter((collection) => collection.parentId === selected.id).length}
              sharedSkills={app.skills ?? []}
              onUpdate={(changes) => send({ type: "update_collection", id: selected.id, ...changes })}
              onSetup={() => send({ type: "setup_collection", id: selected.id })}
              onRemoveLesson={(lessonId) => send({ type: "remove_lesson", id: selected.id, lessonId })}
              onRemoveExample={(name) => send({ type: "remove_example", id: selected.id, name })}
              onSaveSkill={(skill) => send({ type: "save_skill", id: selected.id, ...skill })}
              onRemoveSkill={(name) => send({ type: "remove_skill", id: selected.id, name })}
              onDelete={() => {
                send({ type: "delete_collection", id: selected.id });
                selectedId = null;
              }}
            />
          {/key}
          <p class="-mt-4 px-1 text-xs text-muted-foreground">
            Each project has a file of its own too: PROJECT.md in the project's folder, with the video's brief, footage notes,
            decisions and feedback. The agent keeps it, and you can edit it. It comes after the rules of its
            {COLLECTION_LABEL.brand.lower}, {COLLECTION_LABEL.subbrand.lower} and {COLLECTION_LABEL.series.lower}.
          </p>
        {:else if rows.length}
          <p class="-mt-4 px-1 text-xs text-muted-foreground">Pick one to rename it, move it, write its rules or set it up with AI.</p>
        {/if}
      </div>
    </div>
  </main>
</div>
