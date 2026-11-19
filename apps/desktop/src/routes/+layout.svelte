<script lang="ts">
  import "../app.css";
  import CircleAlert from "@lucide/svelte/icons/circle-alert";
  import WebStart from "#lib/components/WebStart.svelte";
  import { web } from "#lib/web.svelte.ts";

  let { children } = $props();

  // A computer serving this page answers /api/hello; on a web address, the start screen picks one first.
  void web.start();
</script>

{#if web.phase.kind === "host"}
  {@render children()}
{:else if web.phase.kind === "connected"}
  <div class="flex h-full flex-col">
    {#if web.versionHint}
      <div class="flex shrink-0 items-center justify-center gap-2 border-b bg-warning/10 px-4 py-1.5 text-xs text-warning-foreground">
        <CircleAlert class="size-3.5" />
        {web.versionHint}
      </div>
    {/if}
    <div class="min-h-0 flex-1">{@render children()}</div>
  </div>
{:else if web.phase.kind === "checking"}
  <div class="title-bar h-full"></div>
{:else}
  <WebStart />
{/if}
