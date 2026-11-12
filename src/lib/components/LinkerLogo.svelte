<script lang="ts">
  // Linker's logo, as the Linker app draws it in its wordmark
  // (apps/app/src/lib/components/Wordmark.svelte): two cards in an L, lit pink
  // from the top left and blue from the bottom right. Each colour is a copy of
  // the cards, offset behind them; at this size a blur filter would be under a
  // pixel. Drawn on the logo file's 800 × 800 canvas, the viewBox crops it to
  // the mark. The cards take the text colour, so it works in both themes.
  let { class: className = "" }: { class?: string } = $props();

  // Back to front: the blue glow, the pink glow, the cards.
  const layers = [
    { fill: "#4545e6", offset: 14 },
    { fill: "#a01d90", offset: -14 },
    { fill: "currentColor", offset: 0 },
  ];
</script>

<svg class={className} viewBox="170 170 460 460" aria-hidden="true" focusable="false">
  {#each layers as { fill, offset } (fill)}
    <g {fill} transform="translate({offset} {offset})">
      <rect x="198" y="192" width="154" height="275" rx="19" />
      <rect x="353" y="468" width="249" height="140" rx="19" />
    </g>
  {/each}
</svg>
