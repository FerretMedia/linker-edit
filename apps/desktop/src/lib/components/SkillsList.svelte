<script lang="ts">
  import FileUp from "@lucide/svelte/icons/file-up";
  import FolderUp from "@lucide/svelte/icons/folder-up";
  import Upload from "@lucide/svelte/icons/upload";
  import X from "@lucide/svelte/icons/x";
  import type { SavedSkill, SkillImportResult, SkippedFile } from "#shared/protocol.ts";
  import { droppedFiles, importSummary, pickedFiles, readForImport, type PickedFile } from "#lib/skillFiles.ts";

  // Skills of a brand, sub-brand or series, or your own: the list, and skill files dropped on it or picked to import.
  let {
    skills,
    shared,
    about,
    onImport,
    onRemove,
  }: {
    skills: SavedSkill[];
    /** The skills every project gets: one of the same name here adds to it. */
    shared: { name: string }[];
    /** What these skills are for, under the heading. */
    about: string;
    onImport: (files: { path: string; base64: string }[]) => Promise<SkillImportResult | { error: string }>;
    onRemove: (name: string) => void;
  } = $props();

  let over = $state(false);
  // dragenter and dragleave come for every element inside too.
  let depth = 0;
  let busy = $state(false);
  let menu = $state(false);
  let notice = $state<{ text: string; error: boolean; skipped: SkippedFile[] } | null>(null);
  let root: HTMLDivElement;
  let fileInput: HTMLInputElement;
  let folderInput: HTMLInputElement;

  const carriesFiles = (event: DragEvent) => !!event.dataTransfer?.types.includes("Files");

  async function importFiles(picked: Promise<PickedFile[]>) {
    busy = true;
    notice = null;
    try {
      const read = await readForImport(await picked);
      if ("error" in read) {
        notice = { text: read.error, error: true, skipped: [] };
        return;
      }
      const result = read.files.length ? await onImport(read.files) : { imported: [], replaced: [], skipped: [] };
      if ("error" in result) {
        notice = { text: result.error, error: true, skipped: [] };
        return;
      }
      const all = { ...result, skipped: [...read.skipped, ...result.skipped] };
      notice = { text: importSummary(all), error: all.imported.length === 0, skipped: all.skipped };
    } catch (error) {
      notice = { text: `Couldn't read what was dropped: ${error instanceof Error ? error.message : String(error)}`, error: true, skipped: [] };
    } finally {
      busy = false;
    }
  }

  function picked(event: Event & { currentTarget: HTMLInputElement }) {
    const files = pickedFiles(event.currentTarget.files ?? new DataTransfer().files);
    event.currentTarget.value = "";
    if (files.length) void importFiles(Promise.resolve(files));
  }

  function remove(skill: SavedSkill) {
    if (!confirm(`Remove the skill "${skill.name}"? Its folder is deleted, with any other files in it.`)) return;
    onRemove(skill.name);
  }

  const button = "h-8 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
  const item = "flex h-8 w-full items-center gap-2 rounded-md px-2 text-left text-sm transition-colors hover:bg-accent";
</script>

<svelte:window
  onpointerdown={(event) => menu && !root.contains(event.target as Node) && (menu = false)}
  onkeydown={(event) => event.key === "Escape" && (menu = false)}
/>

<div
  bind:this={root}
  class="flex flex-col gap-2 rounded-xl p-4 outline-2 -outline-offset-4 transition-colors {over ? 'bg-primary/5 outline-primary outline-dashed' : 'outline-transparent'}"
  role="region"
  aria-label="Skills"
  ondragenter={(event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth += 1;
    over = true;
  }}
  ondragover={(event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    event.dataTransfer!.dropEffect = "copy";
  }}
  ondragleave={() => {
    depth = Math.max(0, depth - 1);
    if (depth === 0) over = false;
  }}
  ondrop={(event) => {
    if (!carriesFiles(event)) return;
    event.preventDefault();
    depth = 0;
    over = false;
    void importFiles(droppedFiles(event.dataTransfer!));
  }}
>
  <div class="flex items-center gap-2">
    <p class="flex-1 text-sm">Skills</p>
    <div class="relative">
      <button type="button" class="{button} flex items-center gap-1.5" aria-haspopup="menu" aria-expanded={menu} disabled={busy} onclick={() => (menu = !menu)}>
        <Upload class="size-3.5" />
        {busy ? "Importing…" : "Import"}
      </button>
      {#if menu}
        <div class="absolute top-full right-0 z-50 mt-1 w-40 rounded-lg border bg-card p-1 shadow-lg shadow-black/10" role="menu">
          <button type="button" role="menuitem" class={item} onclick={() => ((menu = false), fileInput.click())}>
            <FileUp class="size-3.5 shrink-0" />
            Files
          </button>
          <button type="button" role="menuitem" class={item} onclick={() => ((menu = false), folderInput.click())}>
            <FolderUp class="size-3.5 shrink-0" />
            A folder
          </button>
        </div>
      {/if}
    </div>
    <input bind:this={fileInput} type="file" class="hidden" multiple accept=".md,.zip,.skill" aria-label="Import skill files" onchange={picked} />
    <input bind:this={folderInput} type="file" class="hidden" webkitdirectory aria-label="Import a folder of skills" onchange={picked} />
  </div>
  <p class="text-xs text-muted-foreground">{about}</p>

  {#if notice}
    <div class="flex items-start gap-2 rounded-md border px-2.5 py-2 text-xs {notice.error ? 'border-destructive/40 text-destructive' : ''}" role="status">
      <div class="min-w-0 flex-1">
        <p>{notice.text}</p>
        {#if notice.skipped.length}
          <details class="mt-1 text-muted-foreground">
            <summary class="cursor-pointer select-none">What was skipped</summary>
            <ul class="mt-1 flex flex-col gap-1">
              {#each notice.skipped as skip (skip.path)}
                <li class="select-text"><code class="font-mono text-2xs break-all text-foreground">{skip.path}</code> · {skip.reason}</li>
              {/each}
            </ul>
          </details>
        {/if}
      </div>
      <button
        type="button"
        class="flex size-6 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        aria-label="Dismiss"
        onclick={() => (notice = null)}
      >
        <X class="size-3.5" />
      </button>
    </div>
  {/if}

  {#if skills.length}
    <ul class="divide-y rounded-md border bg-card">
      {#each skills as skill (skill.name)}
        <li class="flex items-center gap-2 px-2.5 py-1.5">
          <div class="min-w-0 flex-1" title={skill.file}>
            <p class="flex items-center gap-2 text-sm">
              <span class="truncate">{skill.name}</span>
              {#if shared.some((other) => other.name === skill.name)}
                <span class="shrink-0 rounded-sm bg-muted px-1.5 py-0.5 text-2xs text-muted-foreground">Adds to the one every project gets</span>
              {/if}
            </p>
            {#if skill.description}<p class="truncate text-xs text-muted-foreground">{skill.description}</p>{/if}
          </div>
          <button
            type="button"
            class="flex size-7 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-destructive"
            aria-label="Remove the skill {skill.name}"
            title="Remove this skill"
            onclick={() => remove(skill)}
          >
            <X class="size-3.5" />
          </button>
        </li>
      {/each}
    </ul>
  {/if}
  <p class="rounded-md border border-dashed px-3 py-3 text-center text-xs text-muted-foreground {over ? 'border-primary text-foreground' : ''}">
    {over ? "Drop to import" : "Drop a SKILL.md, a skill's folder, a folder of skills, or a .zip or .skill file here. A skill of the same name is replaced."}
  </p>
</div>
