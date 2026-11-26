<script lang="ts">
  import { untrack } from "svelte";
  import Sparkles from "@lucide/svelte/icons/sparkles";
  import Trash2 from "@lucide/svelte/icons/trash";
  import X from "@lucide/svelte/icons/x";
  import type { CollectionIcon as Icon, CollectionSummary } from "#shared/protocol.ts";
  import { childWords, collectionLabel, NOT_IN_A_BRAND } from "#lib/labels.ts";
  import CollectionIcon from "./CollectionIcon.svelte";
  import CollectionIconPicker from "./CollectionIconPicker.svelte";
  import SettingsGroup from "./SettingsGroup.svelte";
  import SettingsRow from "./SettingsRow.svelte";
  import SkillsList from "./SkillsList.svelte";
  import Switch from "./Switch.svelte";

  // One brand, sub-brand or series: its name, icon, place and rules. Keyed by id where it's used, so drafts start fresh.
  let {
    collection,
    parents,
    projectNames,
    childCount,
    sharedSkills,
    onUpdate,
    onSetup,
    onRemoveLesson,
    onRemoveExample,
    onSaveSkill,
    onRemoveSkill,
    onDelete,
  }: {
    collection: CollectionSummary;
    /** Where it can go, as PARENT_KINDS allows. */
    parents: { id: string; label: string; disabled: boolean }[];
    projectNames: string[];
    /** The sub-brands and series directly in it. */
    childCount: number;
    /** The skills every project gets, which this one's can add to. */
    sharedSkills: { name: string; description: string }[];
    onUpdate: (changes: { name?: string; icon?: Icon; rules?: string; parentId?: string | null; autonomous?: boolean }) => void;
    /** Opens its setup thread, where the agent studies example videos and proposes the rules. */
    onSetup: () => void;
    onRemoveLesson: (lessonId: string) => void;
    onRemoveExample: (name: string) => void;
    onSaveSkill: (skill: { name: string; description: string; text: string; isNew?: boolean }) => void;
    onRemoveSkill: (name: string) => void;
    onDelete: () => void;
  } = $props();

  const label = $derived(collectionLabel(collection.kind));
  // A brand is a Linker org: its name and picture come from Linker, and it stays.
  const brand = $derived(collection.kind === "brand");
  const inside = $derived(childWords(collection.kind));
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
  const empty = $derived(childCount === 0 && projectNames.length === 0);

  function saveName() {
    const next = name.replace(/\s+/g, " ").trim();
    if (!next) name = collection.name;
    else if (next !== collection.name) onUpdate({ name: next });
  }

  const dateFormat = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric", year: "numeric" });
  // Fewer rounds over time means the brief works.
  const rounds = $derived(collection.edits.slice(0, 8).map((edit) => edit.rounds));

  function remove() {
    if (confirm(`Delete ${label.lower} "${collection.name}"? Its rules, skills, lessons and notes are deleted too.`)) onDelete();
  }

  const button = "h-8 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary sm:w-56";
</script>

<SettingsGroup title={collection.name} description={label.one}>
  {#if brand}
    <SettingsRow label="Linker org" description="Its name and picture come from Linker. Change them there.">
      <CollectionIcon icon={collection.icon} name={collection.name} picture={collection.org?.imageUrl} size="lg" />
    </SettingsRow>
  {:else}
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

    <!-- One from before brands is in none until it is moved. -->
    <SettingsRow label={collection.parentId ? "In" : "Move to"}>
      <select
        class={field}
        aria-label={collection.parentId ? "In" : `Move ${collection.name} to`}
        value={collection.parentId ?? ""}
        onchange={(event) => event.currentTarget.value && onUpdate({ parentId: event.currentTarget.value })}
      >
        {#if !collection.parentId}
          <option value="" disabled>{NOT_IN_A_BRAND}</option>
        {/if}
        {#each parents as parent (parent.id)}
          <option value={parent.id} disabled={parent.disabled}>{parent.label}</option>
        {/each}
      </select>
    </SettingsRow>
  {/if}

  <SettingsRow
    label="Projects"
    description={projectNames.length ? projectNames.join(", ") : "None yet. Put a project in it from Add project or the project's settings."}
  />

  <SettingsRow
    label="Set up with AI"
    description="Point the agent at videos and Resolve projects you've finished. It studies them, asks what it can't see, and proposes these rules for you to accept."
  >
    <button type="button" class="{button} flex items-center gap-1.5" onclick={onSetup}>
      <Sparkles class="size-3.5" />
      {collection.setupProjectId ? "Open setup" : "Set up with AI"}
    </button>
  </SettingsRow>

  <SettingsRow
    label="Works on its own"
    description="Give a project in this {label.lower} footage and the agent makes the whole first cut without asking, then says what it decided. You give feedback on the video."
  >
    <Switch label="Works on its own" checked={collection.autonomous} onChange={(autonomous) => onUpdate({ autonomous })} />
  </SettingsRow>

  <div class="flex flex-col gap-2 p-4">
    <label for="collection-rules" class="text-sm">Rules for the agent</label>
    <p class="text-xs text-muted-foreground">
      In plain words, for every project in this {label.lower}{inside ? ` and its ${inside}` : ""}.
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

  <SkillsList skills={collection.skills ?? []} shared={sharedSkills} of={label.lower} onSave={onSaveSkill} onRemove={onRemoveSkill} />

  <div class="flex flex-col gap-2 p-4">
    <p class="text-sm">Lessons from your feedback</p>
    <p class="text-xs text-muted-foreground">
      {#if collection.lessons.length}
        The agent follows these after the rules above. You accepted each one; remove any that turned out wrong.
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

  {#if !brand}
    <SettingsRow
      label="Delete {label.lower}"
      description={empty
        ? "Its rules, skills, lessons and notes are deleted too. Projects are never deleted."
        : `Move its projects${inside ? ` and ${inside}` : ""} out first.`}
    >
      <button type="button" class="{button} flex items-center gap-1.5 text-destructive" disabled={!empty} onclick={remove}>
        <Trash2 class="size-3.5" />
        Delete
      </button>
    </SettingsRow>
  {/if}
</SettingsGroup>
