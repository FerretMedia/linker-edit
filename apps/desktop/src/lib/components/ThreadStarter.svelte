<script lang="ts">
  import type { FolderListing } from "#shared/protocol.ts";
  import { trimSeparators } from "#lib/browsePath.ts";
  import FolderPicker from "./FolderPicker.svelte";

  // How an empty thread starts when the agent needs folders rather than a question: a setup thread's example
  // videos, or the footage of a video it edits on its own. It writes the first message for the user.
  export interface StarterField {
    key: string;
    label: string;
    hint: string;
    required: boolean;
  }

  let {
    title,
    intro,
    fields,
    notesPlaceholder,
    button,
    disabled,
    browseFolders,
    onStart,
  }: {
    title: string;
    intro: string;
    fields: StarterField[];
    notesPlaceholder: string;
    button: string;
    /** Why it can't start now, as a tooltip; null when it can. */
    disabled: string | null;
    browseFolders: (path: string) => Promise<FolderListing | { error: string }>;
    onStart: (folders: Record<string, string>, notes: string) => void;
  } = $props();

  let folders = $state<Record<string, string>>({});
  let notes = $state("");
  const missing = $derived(fields.some((field) => field.required && !(folders[field.key] ?? "").trim()));

  function start() {
    if (missing || disabled) return;
    onStart(Object.fromEntries(fields.map((field) => [field.key, trimSeparators((folders[field.key] ?? "").trim())])), notes.trim());
  }
</script>

<form
  class="my-auto flex w-full max-w-xl flex-col gap-4 self-center py-10"
  onsubmit={(event) => {
    event.preventDefault();
    start();
  }}
>
  <div class="text-center">
    <h2 class="text-2xl font-semibold tracking-tight">{title}</h2>
    <p class="mt-2 text-sm text-muted-foreground">{intro}</p>
  </div>

  {#each fields as field (field.key)}
    <div class="flex flex-col gap-1">
      <span class="text-sm">
        {field.label}
        {#if !field.required}<span class="text-xs text-muted-foreground">· optional</span>{/if}
      </span>
      <FolderPicker bind:value={() => folders[field.key] ?? "", (value) => (folders[field.key] = value)} browse={browseFolders} listOnFocus label={field.label} />
      <span class="text-xs text-muted-foreground">{field.hint}</span>
    </div>
  {/each}

  <label class="flex flex-col gap-1 text-sm">
    <span>Anything it should know <span class="text-xs text-muted-foreground">· optional</span></span>
    <textarea
      class="field-sizing-content min-h-16 w-full rounded-md border bg-transparent px-2.5 py-1.5 text-sm outline-none focus:border-primary"
      placeholder={notesPlaceholder}
      bind:value={notes}
    ></textarea>
  </label>

  <div class="flex items-center gap-3">
    <p class="min-w-0 flex-1 text-xs text-muted-foreground">Or just write in the box below.</p>
    <button
      type="submit"
      class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
      disabled={missing || disabled !== null}
      title={disabled ?? (missing ? "Pick a folder first." : undefined)}
    >
      {button}
    </button>
  </div>
</form>
