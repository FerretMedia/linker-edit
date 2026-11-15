<script lang="ts">
  import { untrack } from "svelte";
  import { Trash2 } from "@lucide/svelte";
  import type { CollectionIcon as Icon, CollectionSummary } from "#shared/protocol.ts";
  import { COLLECTION_LABEL } from "#lib/labels.ts";
  import CollectionIcon from "./CollectionIcon.svelte";
  import CollectionIconPicker from "./CollectionIconPicker.svelte";
  import SettingsGroup from "./SettingsGroup.svelte";
  import SettingsRow from "./SettingsRow.svelte";

  // One workspace or series: its name, icon, place and rules. Keyed by id where it's used, so drafts start fresh.
  let {
    collection,
    workspaces,
    projectNames,
    seriesCount,
    onUpdate,
    onDelete,
  }: {
    collection: CollectionSummary;
    workspaces: CollectionSummary[];
    projectNames: string[];
    seriesCount: number;
    onUpdate: (changes: { name?: string; icon?: Icon; rules?: string; parentId?: string | null }) => void;
    onDelete: () => void;
  } = $props();

  const label = $derived(COLLECTION_LABEL[collection.kind]);
  const initial = untrack(() => collection);
  let name = $state(initial.name);
  let rules = $state(initial.rules);
  // A change saved elsewhere, as in another window, shows here unless it's being edited here.
  let savedName = initial.name;
  let savedRules = initial.rules;
  $effect(() => {
    const next = { name: collection.name, rules: collection.rules };
    untrack(() => {
      if (name === savedName) name = next.name;
      if (rules === savedRules) rules = next.rules;
      savedName = next.name;
      savedRules = next.rules;
    });
  });
  let picking = $state(false);
  const rulesChanged = $derived(rules !== collection.rules);
  const empty = $derived(seriesCount === 0 && projectNames.length === 0);

  function saveName() {
    const next = name.replace(/\s+/g, " ").trim();
    if (!next) name = collection.name;
    else if (next !== collection.name) onUpdate({ name: next });
  }

  function remove() {
    if (confirm(`Delete ${label.lower} "${collection.name}"? Its rules are deleted too.`)) onDelete();
  }

  const button = "h-8 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary sm:w-56";
</script>

<SettingsGroup title={collection.name} description={label.one}>
  <SettingsRow label="Name">
    <input
      class={field}
      aria-label="{label.one} name"
      maxlength="80"
      bind:value={name}
      onblur={saveName}
      onkeydown={(event) => event.key === "Enter" && event.currentTarget.blur()}
    />
  </SettingsRow>

  <SettingsRow label="Icon">
    {#snippet detail()}
      {#if picking}
        <div class="mt-3">
          <CollectionIconPicker icon={collection.icon} name={collection.name} onChange={(icon) => onUpdate({ icon })} />
        </div>
      {/if}
    {/snippet}
    <CollectionIcon icon={collection.icon} name={collection.name} size="lg" />
    <button type="button" class={button} aria-expanded={picking} onclick={() => (picking = !picking)}>
      {picking ? "Done" : "Change"}
    </button>
  </SettingsRow>

  {#if collection.kind === "series"}
    <SettingsRow label="In {COLLECTION_LABEL.workspace.lower}">
      <select
        class={field}
        aria-label="In {COLLECTION_LABEL.workspace.lower}"
        value={collection.parentId ?? ""}
        onchange={(event) => onUpdate({ parentId: event.currentTarget.value || null })}
      >
        <option value="">None</option>
        {#each workspaces as workspace (workspace.id)}
          <option value={workspace.id}>{workspace.name}</option>
        {/each}
      </select>
    </SettingsRow>
  {/if}

  <SettingsRow
    label="Projects"
    description={projectNames.length
      ? projectNames.join(", ")
      : `None yet. Put a project in it from the project's settings.`}
  />

  <div class="flex flex-col gap-2 p-4">
    <label for="collection-rules" class="text-sm">Rules for the agent</label>
    <p class="text-xs text-muted-foreground">
      In plain words, for every project in this {label.lower}{collection.kind === "workspace" ? ` and its ${COLLECTION_LABEL.series.lowerMany}` : ""}.
      More specific rules come after these and win where they disagree.
    </p>
    <textarea
      id="collection-rules"
      class="min-h-40 w-full resize-y rounded-md border bg-transparent px-2.5 py-2 font-mono text-xs leading-relaxed outline-none focus:border-primary"
      placeholder="Lower thirds in Inter Bold. No swearing. End on the logo."
      bind:value={rules}
      onkeydown={(event) => {
        if (event.key === "s" && (event.ctrlKey || event.metaKey) && rulesChanged) {
          event.preventDefault();
          onUpdate({ rules });
        }
      }}
    ></textarea>
    <div class="flex items-center gap-2">
      <code class="min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground" title={collection.rulesFile}>{collection.rulesFile}</code>
      {#if rulesChanged}
        <button type="button" class={button} onclick={() => (rules = collection.rules)}>Undo</button>
      {/if}
      <button
        type="button"
        class="h-8 rounded-md bg-primary px-3 text-xs whitespace-nowrap text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
        disabled={!rulesChanged}
        onclick={() => onUpdate({ rules })}
      >
        {rulesChanged ? "Save rules" : "Saved"}
      </button>
    </div>
  </div>

  <SettingsRow
    label="Delete {label.lower}"
    description={empty
      ? "Its rules are deleted too. Projects are never deleted."
      : `Move its projects${collection.kind === "workspace" ? ` and ${COLLECTION_LABEL.series.lowerMany}` : ""} out first.`}
  >
    <button type="button" class="{button} flex items-center gap-1.5 text-destructive" disabled={!empty} onclick={remove}>
      <Trash2 class="size-3.5" />
      Delete
    </button>
  </SettingsRow>
</SettingsGroup>
