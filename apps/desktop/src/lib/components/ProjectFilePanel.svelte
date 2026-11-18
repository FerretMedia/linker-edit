<script lang="ts">
  import FileText from "@lucide/svelte/icons/file-text";
  import X from "@lucide/svelte/icons/x";

  let {
    file,
    onSave,
    onClose,
  }: {
    /** PROJECT.md as it is on disk; null while it loads. */
    file: { file: string; text: string } | null;
    onSave: (text: string) => void;
    onClose: () => void;
  } = $props();

  // What the user typed since the text on disk last changed; null follows the disk, so the agent's turns show.
  let draft = $state<string | null>(null);
  let basis = $state<string | null>(null);
  const saved = $derived(file?.text ?? "");
  const text = $derived(draft ?? saved);
  const changed = $derived(draft !== null && draft !== saved);
  // The file changed on disk under an edit: the agent wrote it in a turn.
  const overtaken = $derived(changed && basis !== null && basis !== saved);

  function edit(value: string) {
    if (draft === null) basis = saved;
    draft = value;
  }

  function reset() {
    draft = null;
    basis = null;
  }

  function save() {
    if (!changed) return;
    onSave(text);
    reset();
  }

  const button = "h-8 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent";
</script>

<aside class="flex w-[24rem] shrink-0 flex-col border-l bg-sidebar">
  <header class="title-bar flex h-13 shrink-0 items-center gap-2 border-b px-4">
    <FileText class="size-4 text-muted-foreground" />
    <h2 class="text-sm font-medium">Project file</h2>
    <button
      type="button"
      class="ml-auto flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      onclick={onClose}
      title="Close project file"
    >
      <X class="size-4" />
    </button>
  </header>

  {#if file === null}
    <p class="px-4 py-6 text-center text-xs text-muted-foreground">Loading the project file…</p>
  {:else}
    <div class="flex min-h-0 flex-1 flex-col gap-2 p-4">
      <p class="text-xs text-muted-foreground">
        What this video is: its brief, what is in the footage, decisions, your feedback and where it stands. The agent keeps it
        and reads it in every thread of this project. You can change it too.
      </p>
      <textarea
        class="min-h-0 w-full flex-1 resize-none rounded-md border bg-transparent px-2.5 py-2 font-mono text-xs leading-relaxed outline-none focus:border-primary"
        aria-label="Project file"
        placeholder={"Nothing yet. The agent writes it when it learns what the video is, or start it yourself:\n\n# Working title\n\n## Brief\nWhat the video is about, who it's for, what must be in it."}
        value={text}
        oninput={(event) => edit(event.currentTarget.value)}
        onkeydown={(event) => {
          if (event.key === "s" && (event.ctrlKey || event.metaKey)) {
            event.preventDefault();
            save();
          }
        }}
      ></textarea>
      {#if overtaken}
        <p class="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning-foreground">The agent changed this file while you were editing. Saving replaces its version; Undo shows it.</p>
      {/if}
      <div class="flex items-center gap-2">
        <code class="min-w-0 flex-1 truncate font-mono text-2xs text-muted-foreground" title={file.file}>{file.file}</code>
        {#if changed}
          <button type="button" class={button} onclick={reset}>Undo</button>
        {/if}
        <button
          type="button"
          class="h-8 rounded-md bg-primary px-3 text-xs whitespace-nowrap text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
          disabled={!changed}
          onclick={save}
        >
          {changed ? "Save" : "Saved"}
        </button>
      </div>
    </div>
  {/if}
</aside>
