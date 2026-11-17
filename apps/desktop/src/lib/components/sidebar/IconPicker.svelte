<script lang="ts">
  // T3 Code's project icon picker (apps/web/src/components/settings/ProjectIconPickerDialog.tsx; MIT, see NOTICE):
  // a Lucide icon, an emoji or a monogram, and a color.
  import { untrack, type Component } from "svelte";
  import type { ProjectIcon as Icon, ProjectIconColor } from "#shared/protocol.ts";
  import { deriveProjectIdentity, firstEmoji, monogramText } from "#shared/project-icon.ts";
  import { iconByName, iconNames, loadLucide } from "#lib/lucideIcons.ts";
  import { PROJECT_EMOJIS, PROJECT_ICON_COLOR_OPTIONS, POPULAR_PROJECT_ICONS, colorClass } from "#lib/projectIconColors.ts";
  import Dialog from "../Dialog.svelte";
  import Segmented from "../Segmented.svelte";
  import ProjectIcon from "./ProjectIcon.svelte";

  let {
    current,
    projectName,
    onSelect,
    onClose,
  }: { current: Icon | null; projectName: string; onSelect: (icon: Icon) => void; onClose: () => void } = $props();

  const start = untrack(() => current);
  const automatic = untrack(() => deriveProjectIdentity(projectName));
  let mode = $state<Icon["kind"]>(start?.kind ?? "lucide");
  let iconName = $state(start?.kind === "lucide" ? start.name : "folder-code");
  let color = $state<ProjectIconColor>(start && start.kind !== "emoji" ? start.color : automatic.color);
  let letters = $state(start?.kind === "monogram" ? start.text : automatic.monogram);
  let emoji = $state(start?.kind === "emoji" ? start.emoji : "🎬");
  let query = $state("");
  let customEmoji = $state("");

  const monogram = $derived(monogramText(letters));

  function filterIcons(lucide: Awaited<ReturnType<typeof loadLucide>>): { name: string; Icon: Component<{ class?: string }> }[] {
    const normalized = query.trim().toLowerCase().replaceAll(/\s+/g, "-");
    const names = normalized ? iconNames(lucide).filter((name) => name.includes(normalized)).slice(0, 60) : POPULAR_PROJECT_ICONS;
    return names.flatMap((name) => {
      const Icon = iconByName(lucide, name);
      return Icon ? [{ name, Icon }] : [];
    });
  }

  const label = (name: string) =>
    name
      .split("-")
      .map((part) => part.charAt(0).toUpperCase() + part.slice(1))
      .join(" ");

  function save() {
    if (mode === "monogram" && !monogram) return;
    onSelect(
      mode === "monogram" ? { kind: "monogram", text: monogram!, color } : mode === "lucide" ? { kind: "lucide", name: iconName, color } : { kind: "emoji", emoji },
    );
    onClose();
  }

  const tile =
    "flex aspect-square items-center justify-center rounded-md border border-transparent outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset aria-pressed:border-border aria-pressed:bg-accent";
  const field = "h-8 w-full rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
</script>

<Dialog title="Choose project icon" {onClose}>
  <p class="-mt-1 text-sm text-muted-foreground">Choose an icon, emoji, or monogram.</p>
  <Segmented
    label="Icon type"
    options={[
      { value: "lucide", label: "Icons" },
      { value: "emoji", label: "Emoji" },
      { value: "monogram", label: "Monogram" },
    ]}
    value={mode}
    onChange={(next) => (mode = next)}
  />

  {#if mode !== "emoji"}
    <div>
      <div class="mb-2 text-xs font-medium text-muted-foreground">Color</div>
      <div class="flex flex-wrap gap-1.5" role="group" aria-label="Icon color">
        {#each PROJECT_ICON_COLOR_OPTIONS as option (option.value)}
          <button
            type="button"
            aria-label={option.label}
            aria-pressed={color === option.value}
            class="flex size-6 items-center justify-center rounded-full border border-transparent outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset aria-pressed:border-foreground/64"
            onclick={() => (color = option.value)}
          >
            <span class="size-4 rounded-full {option.swatchClassName}"></span>
          </button>
        {/each}
      </div>
    </div>
  {/if}

  {#if mode === "lucide"}
    <input type="search" class={field} bind:value={query} aria-label="Search Lucide icons" placeholder="Search all Lucide icons" />
    {#await loadLucide()}
      <p class="py-8 text-center text-sm text-muted-foreground">Loading icons…</p>
    {:then lucide}
      {@const icons = filterIcons(lucide)}
      <div class="max-h-64 overflow-y-auto">
        <div class="grid grid-cols-10 gap-1 p-0.5">
          {#each icons as icon (icon.name)}
            <button
              type="button"
              aria-label={label(icon.name)}
              title={label(icon.name)}
              aria-pressed={iconName === icon.name}
              class="{tile} {colorClass(color)}"
              onclick={() => (iconName = icon.name)}
            >
              <icon.Icon class="size-5" />
            </button>
          {/each}
        </div>
      </div>
      {#if icons.length === 0}
        <p class="py-8 text-center text-sm text-muted-foreground">No icons found.</p>
      {/if}
    {/await}
  {:else if mode === "monogram"}
    <div class="flex items-center gap-4 py-2">
      <ProjectIcon icon={{ kind: "monogram", text: monogram ?? automatic.monogram, color }} name={projectName} class="size-12" />
      <div class="flex flex-1 flex-col gap-2">
        <label for="project-monogram" class="text-sm font-medium">Letters</label>
        <input id="project-monogram" class={field} bind:value={letters} aria-invalid={!monogram} autocomplete="off" />
        <p class="text-xs text-muted-foreground">One or two letters or numbers.</p>
      </div>
    </div>
  {:else}
    <div class="max-h-64 overflow-y-auto">
      <div class="grid grid-cols-10 gap-1 p-0.5">
        {#each PROJECT_EMOJIS as option (option.emoji)}
          <button type="button" aria-label={option.label} aria-pressed={emoji === option.emoji} class="{tile} text-xl" onclick={() => (emoji = option.emoji)}>
            {option.emoji}
          </button>
        {/each}
      </div>
    </div>
    <div>
      <div class="mb-2 text-xs font-medium text-muted-foreground">Or paste any emoji</div>
      <input
        class={field}
        value={customEmoji}
        aria-label="Custom emoji"
        placeholder="Paste an emoji"
        oninput={(event) => {
          customEmoji = event.currentTarget.value;
          const next = firstEmoji(customEmoji);
          if (next) emoji = next;
        }}
      />
    </div>
  {/if}

  <div class="flex items-center justify-end gap-2 border-t pt-4">
    <button type="button" class="h-8 rounded-md border px-3 text-sm transition-colors hover:bg-accent" onclick={onClose}>Cancel</button>
    <button
      type="button"
      class="h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground transition-opacity hover:opacity-90 disabled:opacity-50"
      disabled={mode === "monogram" && !monogram}
      onclick={save}
    >
      Save icon
    </button>
  </div>
</Dialog>
