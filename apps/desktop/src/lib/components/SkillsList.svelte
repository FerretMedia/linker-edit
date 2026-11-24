<script lang="ts">
  import Plus from "@lucide/svelte/icons/plus";
  import X from "@lucide/svelte/icons/x";
  import type { SavedSkill } from "#shared/protocol.ts";

  // A workspace's or series' own skills: the list, and one open to read or change.
  let {
    skills,
    shared,
    of,
    onSave,
    onRemove,
  }: {
    skills: SavedSkill[];
    /** The skills every project gets: one of the same name here adds to it. */
    shared: { name: string; description: string }[];
    /** "workspace" or "series". */
    of: string;
    onSave: (skill: { name: string; description: string; text: string; isNew?: boolean }) => void;
    onRemove: (name: string) => void;
  } = $props();

  /** The skill open for editing; isNew until the server has it. */
  let draft = $state<{ name: string; description: string; text: string; isNew: boolean } | null>(null);
  const saved = $derived(draft && !draft.isNew ? skills.find((skill) => skill.name === draft!.name) : undefined);
  const changed = $derived(!!draft && (!saved || saved.description !== draft.description || saved.text.trim() !== draft.text.trim()));
  const base = $derived(draft ? shared.find((skill) => skill.name === draft!.name.trim().toLowerCase()) : undefined);
  const suggestions = $derived(shared.filter((skill) => !skills.some((own) => own.name === skill.name)));

  // Closes once the server has what was sent. A refusal keeps the draft, with the reason in a notice.
  let sent: { description: string; text: string } | null = null;
  const plain = (skill: { description: string; text: string }) => ({ description: skill.description.replace(/\s+/g, " ").trim(), text: skill.text.trim() });
  $effect(() => {
    const has = skills.map(plain);
    if (sent && has.some((skill) => skill.text === sent!.text && skill.description === sent!.description)) {
      sent = null;
      draft = null;
    }
  });

  function save() {
    if (!draft || !changed) return;
    sent = plain(draft);
    onSave({ name: draft.name, description: draft.description, text: draft.text, ...(draft.isNew ? { isNew: true } : {}) });
  }

  function remove(skill: SavedSkill) {
    if (!confirm(`Remove the skill "${skill.name}"? Its folder is deleted, with any other files in it.`)) return;
    if (draft?.name === skill.name) draft = null;
    onRemove(skill.name);
  }

  const button = "h-8 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
</script>

{#snippet editor()}
  {#if draft}
    <div class="flex flex-col gap-2 p-2.5">
      {#if draft.isNew}
        <input class={field} list="shared-skills" aria-label="Skill name" placeholder="Name, such as color-grading" maxlength="40" bind:value={draft.name} />
        <datalist id="shared-skills">
          {#each suggestions as skill (skill.name)}<option value={skill.name}></option>{/each}
        </datalist>
      {/if}
      {#if base}
        <p class="text-xs text-muted-foreground">
          Adds to the {base.name} skill every project gets, so write only what is different here.
        </p>
      {/if}
      <input
        class={field}
        aria-label="What the skill is for"
        placeholder={base ? "What work it is for · optional here" : "What work it is for, so the agent knows when to read it"}
        maxlength="300"
        bind:value={draft.description}
      />
      <textarea
        class="min-h-40 w-full resize-y rounded-md border bg-transparent px-2.5 py-2 font-mono text-xs leading-relaxed outline-none focus:border-primary"
        aria-label="Skill text"
        maxlength="20000"
        placeholder="Warm highlights, skin never orange. The hero shot of a scene sets the rest."
        bind:value={draft.text}
        onkeydown={(event) => {
          if (event.key === "s" && (event.ctrlKey || event.metaKey)) {
            event.preventDefault();
            save();
          }
        }}
      ></textarea>
      <div class="flex items-center gap-2">
        <code class="min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground" title={saved?.file}>{saved?.file ?? ""}</code>
        <button type="button" class={button} onclick={() => (draft = null)}>{changed ? "Cancel" : "Close"}</button>
        <button
          type="button"
          class="h-8 rounded-md bg-primary px-3 text-xs whitespace-nowrap text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
          disabled={!changed || !draft.name.trim() || !draft.text.trim()}
          onclick={save}
        >
          Save skill
        </button>
      </div>
    </div>
  {/if}
{/snippet}

<div class="flex flex-col gap-2 p-4">
  <div class="flex items-center gap-2">
    <p class="flex-1 text-sm">Skills</p>
    <button
      type="button"
      class="{button} flex items-center gap-1.5"
      disabled={draft?.isNew}
      onclick={() => (draft = { name: "", description: "", text: "", isNew: true })}
    >
      <Plus class="size-3.5" />
      Add skill
    </button>
  </div>
  <p class="text-xs text-muted-foreground">
    How one kind of work is done in this {of}, such as color grading. The agent reads a skill only when the work needs it; the rules
    above it reads always.
  </p>
  {#if skills.length || draft?.isNew}
    <ul class="divide-y rounded-md border">
      {#if draft?.isNew}
        <li>{@render editor()}</li>
      {/if}
      {#each skills as skill (skill.name)}
        <li>
          <div class="flex items-center gap-2 px-2.5 py-1.5">
            <button
              type="button"
              class="min-w-0 flex-1 text-left"
              aria-expanded={!draft?.isNew && draft?.name === skill.name}
              onclick={() => (draft = !draft?.isNew && draft?.name === skill.name ? null : { name: skill.name, description: skill.description, text: skill.text, isNew: false })}
            >
              <span class="block truncate text-sm">{skill.name}</span>
              {#if skill.description}<span class="block truncate text-xs text-muted-foreground">{skill.description}</span>{/if}
            </button>
            <button
              type="button"
              class="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
              aria-label="Remove the skill {skill.name}"
              title="Remove this skill"
              onclick={() => remove(skill)}
            >
              <X class="size-3.5" />
            </button>
          </div>
          {#if !draft?.isNew && draft?.name === skill.name}{@render editor()}{/if}
        </li>
      {/each}
    </ul>
  {/if}
</div>
