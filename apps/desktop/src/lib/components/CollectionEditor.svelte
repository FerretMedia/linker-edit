<script lang="ts">
  import { untrack } from "svelte";
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import Trash2 from "@lucide/svelte/icons/trash";
  import X from "@lucide/svelte/icons/x";
  import type { CollectionIcon as Icon, CollectionSummary } from "#shared/protocol.ts";
  import { COLLECTION_LABEL } from "#lib/labels.ts";
  import CollectionIcon from "./CollectionIcon.svelte";
  import CollectionIconPicker from "./CollectionIconPicker.svelte";
  import SettingsGroup from "./SettingsGroup.svelte";
  import SettingsRow from "./SettingsRow.svelte";
  import Switch from "./Switch.svelte";

  // One workspace or series: its name, icon, place and skill. Keyed by id where it's used, so drafts start fresh.
  let {
    collection,
    workspaces,
    projectNames,
    seriesCount,
    onUpdate,
    onSetup,
    onRemoveLesson,
    onRemoveExample,
    onDelete,
  }: {
    collection: CollectionSummary;
    workspaces: CollectionSummary[];
    projectNames: string[];
    seriesCount: number;
    onUpdate: (changes: { name?: string; icon?: Icon; rules?: string; parentId?: string | null; autonomous?: boolean }) => void;
    /** Opens its skill thread, where the agent studies example videos and projects and proposes the skill. */
    onSetup: () => void;
    onRemoveLesson: (lessonId: string) => void;
    onRemoveExample: (name: string) => void;
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

  const dateFormat = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });
  // Fewer rounds over time means the brief works.
  const rounds = $derived(collection.edits.slice(0, 8).map((edit) => edit.rounds));

  function remove() {
    if (confirm(`Delete ${label.lower} "${collection.name}"? Its skill, lessons and notes are deleted too.`)) onDelete();
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

  <SettingsRow
    label="Make the skill with AI"
    description="A thread where the agent studies finished videos and Resolve projects, asks what it can't see, and proposes the skill below for you to accept."
  >
    <button type="button" class="{button} flex items-center gap-1.5" onclick={onSetup}>
      <Sparkles class="size-3.5" />
      {collection.setupProjectId ? "Open skill thread" : "Make the skill with AI"}
    </button>
  </SettingsRow>

  <SettingsRow
    label="Works on its own"
    description="Give a project in this {label.lower} footage and the agent makes the whole first cut without asking, then says what it decided. You give feedback on the video."
  >
    <Switch label="Works on its own" checked={collection.autonomous} onChange={(autonomous) => onUpdate({ autonomous })} />
  </SettingsRow>

  <div class="flex flex-col gap-2 p-4">
    <label for="collection-rules" class="text-sm">Skill (SKILL.md)</label>
    <p class="text-xs text-muted-foreground">
      In plain words, for every project in this {label.lower}{collection.kind === "workspace" ? ` and its ${COLLECTION_LABEL.series.lowerMany}` : ""}.
      A more specific skill comes after this one and wins where they disagree.
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
        {rulesChanged ? "Save skill" : "Saved"}
      </button>
    </div>
  </div>

  <div class="flex flex-col gap-2 p-4">
    <p class="text-sm">Lessons from your feedback</p>
    <p class="text-xs text-muted-foreground">
      {#if collection.lessons.length}
        The agent follows these after the skill above. You accepted each one; remove any that turned out wrong.
      {:else}
        None yet. When your feedback on an edit holds for the next video too, the agent proposes a lesson and you accept or decline it.
      {/if}
      {#if rounds.length}
        Rounds of feedback before you approved an edit, latest first: {rounds.join(" · ")}.
      {/if}
    </p>
    {#if collection.lessons.length}
      <ul class="divide-y rounded-md border">
        {#each collection.lessons as lesson (lesson.id)}
          <li class="flex items-start gap-2 px-2.5 py-2">
            <div class="min-w-0 flex-1">
              <p class="text-sm select-text">{lesson.rule}</p>
              <p class="mt-0.5 text-xs text-muted-foreground select-text">
                From “{lesson.feedback}”{lesson.project ? ` · ${lesson.project}` : ""} · {dateFormat.format(new Date(lesson.addedAt))}
              </p>
            </div>
            <button
              type="button"
              class="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
              aria-label="Remove lesson: {lesson.rule}"
              title="Remove this lesson"
              onclick={() => onRemoveLesson(lesson.id)}
            >
              <X class="size-3.5" />
            </button>
          </li>
        {/each}
      </ul>
    {/if}
  </div>

  {#if collection.examples.length}
    <div class="flex flex-col gap-2 p-4">
      <p class="text-sm">Example videos it studied</p>
      <p class="text-xs text-muted-foreground">The agent's notes on each, kept as files it reads when it needs a reference.</p>
      <ul class="divide-y rounded-md border">
        {#each collection.examples as example (example.name)}
          <li class="flex items-center gap-2 px-2.5 py-1.5">
            <code class="min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground" title={example.file}>{example.file}</code>
            <button
              type="button"
              class="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
              aria-label="Remove the notes on {example.name}"
              title="Remove these notes. The video stays."
              onclick={() => onRemoveExample(example.name)}
            >
              <X class="size-3.5" />
            </button>
          </li>
        {/each}
      </ul>
    </div>
  {/if}

  <SettingsRow
    label="Delete {label.lower}"
    description={empty
      ? "Its skill, lessons and notes are deleted too. Projects are never deleted."
      : `Move its projects${collection.kind === "workspace" ? ` and ${COLLECTION_LABEL.series.lowerMany}` : ""} out first.`}
  >
    <button type="button" class="{button} flex items-center gap-1.5 text-destructive" disabled={!empty} onclick={remove}>
      <Trash2 class="size-3.5" />
      Delete
    </button>
  </SettingsRow>
</SettingsGroup>
