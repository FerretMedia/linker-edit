<script lang="ts">
  import CircleAlert from "@lucide/svelte/icons/circle-alert";
  import LoaderCircle from "@lucide/svelte/icons/loader-circle";
  import LogOut from "@lucide/svelte/icons/log-out";
  import Monitor from "@lucide/svelte/icons/monitor";
  import Network from "@lucide/svelte/icons/network";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import { releaseName } from "#shared/release-name.ts";
  import { web } from "#lib/web.svelte.ts";
  import LinkerLogo from "./LinkerLogo.svelte";

  // The page on someone's own web address: sign in with Linker, then pick one of their computers.
  const button = "flex h-8 items-center gap-1.5 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
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
        <p class="text-sm text-muted-foreground">Sign in with Linker to open one of your edit computers here.</p>
        <button type="button" class={primaryButton} onclick={() => void web.signIn()}>
          <LinkerLogo class="size-4" />
          Sign in with Linker
        </button>
        {#if web.phase.problem}
          <p class="flex items-start gap-1.5 text-xs text-destructive"><CircleAlert class="mt-0.5 size-3.5 shrink-0" />{web.phase.problem}</p>
        {/if}
      </div>
    {:else}
      <section class="flex flex-col gap-2">
        <div class="flex items-center gap-2 px-1">
          <h2 class="flex-1 text-sm font-medium">Your computers</h2>
          <button
            type="button"
            class="flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            aria-label="Look again"
            title="Look again"
            onclick={() => void web.list()}
          >
            <RefreshCw class="size-3.5" />
          </button>
          <button type="button" class={button} onclick={() => web.signOut()}>
            <LogOut class="size-3.5" />
            Sign out
          </button>
        </div>

        {#if web.listProblem}
          <p class="flex items-start gap-1.5 px-1 text-xs text-destructive"><CircleAlert class="mt-0.5 size-3.5 shrink-0" />{web.listProblem}</p>
        {/if}

        {#if web.askNetwork}
          <div class="flex flex-col items-start gap-3 rounded-xl border bg-card p-4">
            <Network class="size-5 text-muted-foreground" />
            <p class="text-sm">
              Your browser may ask to let this page reach devices on your local network. Allow it: that's how it reaches your computers over
              Tailscale. If you don't, it uses the Linker Connect tunnel.
            </p>
            <button type="button" class={primaryButton} onclick={() => web.continueToNetwork()}>Continue</button>
          </div>
        {:else if web.computers === null}
          {#if !web.listProblem}
            <p class="flex items-center gap-2 px-1 text-xs text-muted-foreground">
              <LoaderCircle class="size-3.5 animate-spin" /> Looking for your computers…
            </p>
          {/if}
        {:else if web.computers.length === 0}
          <p class="px-1 text-xs text-muted-foreground">
            No computers yet. On the computer with Resolve, sign in to the same Linker account in Linker-Edit and turn on Linker Connect or
            Tailscale remote access in its settings.
          </p>
        {:else}
          <div class="divide-y rounded-xl border bg-card">
            {#each web.computers as row (row.computer.id)}
              <button
                type="button"
                class="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors first:rounded-t-xl last:rounded-b-xl hover:bg-accent disabled:hover:bg-transparent"
                disabled={row.state !== "ready"}
                onclick={() => void web.pick(row.computer.id)}
              >
                <Monitor class="size-4 shrink-0 text-muted-foreground" />
                <span class="min-w-0 flex-1">
                  <span class="block truncate text-sm {row.state === 'offline' ? 'text-muted-foreground' : ''}">{row.computer.name}</span>
                  {#if row.problem}
                    <span class="block text-xs text-destructive">{row.problem}</span>
                  {:else if row.reached}
                    <span class="block text-xs text-muted-foreground">
                      {row.reached.via === "tailscale" ? "Over Tailscale" : "Through Linker Connect"} · {releaseName(row.reached.hello.version)}
                    </span>
                  {/if}
                </span>
                {#if row.state === "checking" || row.state === "connecting"}
                  <LoaderCircle class="size-3.5 shrink-0 animate-spin text-muted-foreground" />
                {:else if row.state === "offline"}
                  <span class="shrink-0 text-2xs text-muted-foreground" title="Linker-Edit isn't running there, or it can't be reached from here">
                    offline
                  </span>
                {/if}
              </button>
            {/each}
          </div>
        {/if}
      </section>
    {/if}
  </div>
</div>
