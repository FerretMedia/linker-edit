<script lang="ts">
  import type { FolderListing, VideoBrief } from "#shared/protocol.ts";
  import { trimSeparators } from "#lib/browsePath.ts";
  import PathField from "./PathField.svelte";

  // Starts a video from its brief: the server writes it to the project's files and the agent makes the first cut.
  let {
    disabled,
    browseFolders,
    onStart,
    onBack,
  }: {
    /** Why it can't start now, as a tooltip; null when it can. */
    disabled: string | null;
    browseFolders: (path: string) => Promise<FolderListing | { error: string }>;
    onStart: (brief: VideoBrief) => void;
    /** Closes the form; absent where the thread starts with it. */
    onBack?: () => void;
  } = $props();

  let brief = $state<VideoBrief>({ title: "", footage: "", concept: "", script: "" });
  const missing = $derived(!brief.title.trim() || !brief.footage.trim());

  const field = "w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
</script>

{#snippet optional()}<span class="text-xs text-muted-foreground">· optional</span>{/snippet}

<form
  class="my-auto flex w-full max-w-xl flex-col gap-4 self-center py-10"
  onsubmit={(event) => {
    event.preventDefault();
    if (!missing && !disabled) onStart({ ...brief, footage: trimSeparators(brief.footage.trim()) });
  }}
>
  <div class="text-center">
    <h2 class="text-2xl font-semibold tracking-tight">New video</h2>
    <p class="mt-2 text-sm text-muted-foreground">The agent makes the first cut without asking, then tells you what it decided. You give feedback on the video.</p>
  </div>

  <label class="flex flex-col gap-1 text-sm">
    <span>Title</span>
    <input class="h-9 {field}" maxlength="120" bind:value={brief.title} />
  </label>

  <div class="flex flex-col gap-1">
    <span class="text-sm">Footage</span>
    <PathField bind:value={brief.footage} label="Footage" browse={browseFolders} />
    <span class="text-xs text-muted-foreground">The folder with this video's footage.</span>
  </div>

  <label class="flex flex-col gap-1 text-sm">
    <span>Concept {@render optional()}</span>
    <textarea
      class="field-sizing-content min-h-16 py-1.5 {field}"
      maxlength="4000"
      placeholder="What it's about, who it's for, how long, and what must be in it…"
      bind:value={brief.concept}
    ></textarea>
  </label>

  <label class="flex flex-col gap-1 text-sm">
    <span>Script {@render optional()}</span>
    <textarea class="field-sizing-content max-h-72 min-h-16 py-1.5 {field}" maxlength="100000" bind:value={brief.script}></textarea>
  </label>

  <div class="flex items-center gap-3">
    <p class="min-w-0 flex-1 text-xs text-muted-foreground">Or just write in the box below.</p>
    {#if onBack}
      <button type="button" class="h-8 rounded-md border px-3 text-sm transition-colors hover:bg-accent" onclick={onBack}>Back</button>
    {/if}
    <button
      type="submit"
      class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
      disabled={missing || disabled !== null}
      title={disabled ?? (missing ? "Give it a title and a footage folder first." : undefined)}
    >
      Make the first cut
    </button>
  </div>
</form>
