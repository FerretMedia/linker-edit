<script lang="ts">
  import Folder from "@lucide/svelte/icons/folder";
  import type { CollectionIcon } from "#shared/protocol.ts";
  import { ICON_COLOR_CLASS, iconText, LUCIDE_ICONS } from "#lib/collectionIcons.ts";

  // A collection's icon on its color: a lucide icon, an emoji or its letters. A brand shows its Linker org's picture.
  let {
    icon,
    name,
    picture = null,
    size = "md",
  }: { icon: CollectionIcon; name: string; picture?: string | null; size?: "sm" | "md" | "lg" } = $props();

  const box = $derived({ sm: "size-5 rounded text-2xs", md: "size-6 rounded-md text-xs", lg: "size-9 rounded-lg text-sm" }[size]);
  const glyph = $derived({ sm: "size-3", md: "size-3.5", lg: "size-5" }[size]);
  const Lucide = $derived(icon.kind === "lucide" ? (LUCIDE_ICONS[icon.name] ?? Folder) : Folder);
</script>

{#if picture}
  <img src={picture} alt="" referrerpolicy="no-referrer" class="shrink-0 object-cover {box}" />
{:else}
  <span class="flex shrink-0 items-center justify-center font-semibold select-none {box} {ICON_COLOR_CLASS[icon.color]}" aria-hidden="true">
    {#if icon.kind === "lucide"}
      <Lucide class={glyph} />
    {:else if icon.kind === "emoji"}
      <span class="leading-none">{icon.emoji}</span>
    {:else}
      {iconText(icon, name)}
    {/if}
  </span>
{/if}
