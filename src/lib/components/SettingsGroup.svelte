<script lang="ts">
  import type { Snippet } from "svelte";

  // A card of settings rows under a heading, like T3 Code's settings.
  let { title, description, children }: { title: string; description?: string; children: Snippet } = $props();
</script>

<section class="flex flex-col gap-2">
  <div class="px-1">
    <h2 class="text-sm font-medium">{title}</h2>
    {#if description}
      <p class="mt-0.5 text-xs text-muted-foreground">{description}</p>
    {/if}
  </div>
  <div class="divide-y rounded-xl border bg-card">
    {@render children()}
  </div>
</section>
