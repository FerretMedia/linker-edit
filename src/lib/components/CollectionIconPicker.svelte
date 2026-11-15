<script lang="ts">
  import type { CollectionIcon } from "#shared/protocol.ts";
  import { EMOJI_SUGGESTIONS, ICON_COLORS, ICON_SWATCH_CLASS, LUCIDE_ICONS, monogram } from "#lib/collectionIcons.ts";
  import Segmented from "./Segmented.svelte";

  // A simple icon picker for workspaces and series: letters, a lucide icon or an emoji, and a color.
  let { icon, name, onChange }: { icon: CollectionIcon; name: string; onChange: (icon: CollectionIcon) => void } = $props();

  const KINDS = [
    { value: "monogram", label: "Letters" },
    { value: "lucide", label: "Icon" },
    { value: "emoji", label: "Emoji" },
  ] as const;

  function pickKind(kind: CollectionIcon["kind"]) {
    const { color } = icon;
    if (kind === "monogram") onChange({ kind, text: "", color });
    else if (kind === "lucide") onChange({ kind, name: "folder", color });
    else onChange({ kind, emoji: EMOJI_SUGGESTIONS[0]!, color });
  }

  const cell = "flex size-8 items-center justify-center rounded-md transition-colors hover:bg-accent";
  const field = "h-8 w-24 rounded-md border bg-transparent px-2.5 text-sm outline-none focus:border-primary";
</script>

<div class="flex flex-col gap-3">
  <Segmented label="Icon type" options={KINDS} value={icon.kind} onChange={pickKind} />

  {#if icon.kind === "monogram"}
    <label class="flex items-center gap-2 text-xs text-muted-foreground">
      Letters
      <input
        class={field}
        maxlength="3"
        placeholder={monogram(name)}
        value={icon.text}
        oninput={(event) => icon.kind === "monogram" && onChange({ ...icon, text: event.currentTarget.value })}
      />
    </label>
  {:else if icon.kind === "lucide"}
    <div class="flex flex-wrap gap-0.5" role="radiogroup" aria-label="Icon">
      {#each Object.entries(LUCIDE_ICONS) as [lucide, Glyph] (lucide)}
        <button
          type="button"
          role="radio"
          aria-checked={icon.name === lucide}
          aria-label={lucide}
          title={lucide}
          class="{cell} {icon.name === lucide ? 'bg-accent text-foreground ring-1 ring-border' : 'text-muted-foreground'}"
          onclick={() => icon.kind === "lucide" && onChange({ ...icon, name: lucide })}
        >
          <Glyph class="size-4" />
        </button>
      {/each}
    </div>
  {:else}
    <div class="flex flex-wrap items-center gap-0.5">
      {#each EMOJI_SUGGESTIONS as emoji (emoji)}
        <button
          type="button"
          class="{cell} text-base {icon.emoji === emoji ? 'bg-accent ring-1 ring-border' : ''}"
          aria-label={emoji}
          onclick={() => icon.kind === "emoji" && onChange({ ...icon, emoji })}
        >
          {emoji}
        </button>
      {/each}
      <input
        class="{field} ml-1 w-16 text-center"
        aria-label="Emoji"
        maxlength="16"
        value={icon.emoji}
        oninput={(event) => {
          const emoji = event.currentTarget.value.trim();
          if (icon.kind === "emoji" && emoji) onChange({ ...icon, emoji });
        }}
      />
    </div>
  {/if}

  <div class="flex items-center gap-1.5" role="radiogroup" aria-label="Color">
    {#each ICON_COLORS as color (color)}
      <button
        type="button"
        role="radio"
        aria-checked={icon.color === color}
        aria-label={color}
        title={color}
        class="size-5 rounded-full {ICON_SWATCH_CLASS[color]} transition-transform hover:scale-110 {icon.color === color
          ? 'ring-2 ring-foreground ring-offset-2 ring-offset-card'
          : ''}"
        onclick={() => onChange({ ...icon, color })}
      ></button>
    {/each}
  </div>
</div>
