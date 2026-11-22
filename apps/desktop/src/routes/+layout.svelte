<script lang="ts">
  import "../app.css";
  import CircleAlert from "@lucide/svelte/icons/circle-alert";
  import WebStart from "#lib/components/WebStart.svelte";
  import { machines } from "#lib/machines.svelte.ts";
  import { web } from "#lib/web.svelte.ts";

  let { children } = $props();

  // A computer serving this page answers /api/hello; on a web address, signing in comes first.
  void web.start();

  const versionHint = $derived(machines.active ? web.hintFor(machines.active.id) : null);
</script>

{#if web.phase.kind === "host" || web.phase.kind === "app"}
  <div class="flex h-full flex-col">
    {#if versionHint}
      <div class="flex shrink-0 items-center justify-center gap-2 border-b bg-warning/10 px-4 py-1.5 text-xs text-warning-foreground">
        <CircleAlert class="size-3.5" />
        {versionHint}
      </div>
    {/if}
    <div class="min-h-0 flex-1">{@render children()}</div>
  </div>
{:else if web.phase.kind === "checking"}
  <div class="title-bar h-full"></div>
{:else}
  <WebStart />
{/if}
