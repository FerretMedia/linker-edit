<script lang="ts">
  import type { UsageProvider } from "#shared/usage.ts";
  import ClaudeIcon from "../ClaudeIcon.svelte";
  import OpenAIIcon from "../OpenAIIcon.svelte";

  let { provider, class: className = "" }: { provider: UsageProvider; class?: string } = $props();
</script>

{#if provider === "claude"}
  <ClaudeIcon class="shrink-0 {className}" />
{:else}
  <OpenAIIcon class="shrink-0 {className}" />
{/if}
