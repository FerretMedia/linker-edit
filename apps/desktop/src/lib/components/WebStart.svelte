<script lang="ts">
  import CircleAlert from "@lucide/svelte/icons/circle-alert";
  import { web } from "#lib/web.svelte.ts";
  import LinkerLogo from "./LinkerLogo.svelte";

  // The page on someone's own web address, signed out. Signed in, it's the app with all their computers.
  const primaryButton = "flex h-8 items-center gap-1.5 rounded-md bg-primary px-3 text-sm whitespace-nowrap text-primary-foreground transition-opacity hover:opacity-90";
</script>

<div class="flex h-full justify-center overflow-y-auto px-5 py-16">
  <div class="flex w-full max-w-md flex-col gap-6">
    <div class="flex items-center gap-2">
      <LinkerLogo class="size-6 text-foreground" />
      <h1 class="text-lg font-semibold tracking-tight">Linker-Edit</h1>
    </div>

    {#if web.phase.kind === "signed_out"}
      <div class="flex flex-col items-start gap-3">
        <p class="text-sm text-muted-foreground">Sign in with Linker to work on your edit computers here.</p>
        <button type="button" class={primaryButton} onclick={() => void web.signIn()}>
          <LinkerLogo class="size-4" />
          Sign in with Linker
        </button>
        {#if web.phase.problem}
          <p class="flex items-start gap-1.5 text-xs text-destructive"><CircleAlert class="mt-0.5 size-3.5 shrink-0" />{web.phase.problem}</p>
        {/if}
      </div>
    {/if}
  </div>
</div>
