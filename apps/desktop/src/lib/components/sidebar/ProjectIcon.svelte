<script lang="ts">
  // T3 Code's ProjectFavicon and ProjectMonogram (MIT, see NOTICE), without favicons read from the project folder.
  import FolderCode from "@lucide/svelte/icons/folder-code";
  import type { ProjectIcon, ProjectIconColor } from "#shared/protocol.ts";
  import { deriveProjectIdentity } from "#shared/project-icon.ts";
  import { iconByName, loadLucide } from "#lib/lucideIcons.ts";
  import { colorClass } from "#lib/projectIconColors.ts";

  let { icon, name, class: className = "size-4" }: { icon: ProjectIcon | null; name: string; class?: string } = $props();

  const identity = $derived(deriveProjectIdentity(name));
  const segmenter = new Intl.Segmenter(undefined, { granularity: "grapheme" });
</script>

{#snippet monogram(text: string, color: ProjectIconColor)}
  <span aria-hidden="true" data-slot="project-monogram" class="inline-flex shrink-0 items-center justify-center {className}">
    <svg
      viewBox="0 0 16 16"
      class="size-full overflow-hidden rounded-[25%] font-mono select-none {colorClass(color)}"
      style="background-color: color-mix(in srgb, currentColor 14%, transparent)"
    >
      <text
        x="8"
        y="10.8"
        text-anchor="middle"
        fill="currentColor"
        class="font-mono"
        font-size="8.25"
        font-weight="700"
        textLength={Array.from(segmenter.segment(text)).length === 1 ? 6 : 12}
        lengthAdjust="spacingAndGlyphs"
        text-rendering="geometricPrecision">{text}</text
      >
    </svg>
  </span>
{/snippet}

{#if icon?.kind === "monogram"}
  {@render monogram(icon.text, icon.color)}
{:else if icon?.kind === "emoji"}
  <span aria-hidden="true" class="inline-flex shrink-0 items-center justify-center leading-none [container-type:size] {className}">
    <span class="text-[length:80cqh] leading-none">{icon.emoji}</span>
  </span>
{:else if icon?.kind === "lucide"}
  {@const name = icon.name}
  {@const color = colorClass(icon.color)}
  <span aria-hidden="true" class="inline-flex shrink-0 items-center justify-center {className}">
    {#await loadLucide()}
      <FolderCode class="size-full {color}" />
    {:then lucide}
      {@const Icon = iconByName(lucide, name) ?? FolderCode}
      <Icon class="size-full {color}" />
    {/await}
  </span>
{:else}
  {@render monogram(identity.monogram, identity.color)}
{/if}
