<script lang="ts">
  import { ExternalLink, LogIn, LogOut } from "@lucide/svelte";
  import type { LinkerAccount } from "#shared/protocol.ts";
  import RowMenu from "./RowMenu.svelte";

  let {
    linker,
    computer,
    onCancel,
    onSignOut,
  }: { linker: LinkerAccount; computer: string; onCancel: () => void; onSignOut: () => void } = $props();

  // Linker sends the browser back to 127.0.0.1, so signing in only works on the computer Linker-Edit runs on.
  const onThisComputer = ["localhost", "127.0.0.1", "[::1]"].includes(location.hostname);
  // A link, not a script: the desktop app opens it in the system browser, where Linker's sign-in remembers you.
  const SIGN_IN = "/api/linker/sign-in";
</script>

{#if linker.state === "signed_in"}
  <div class="group mx-2 flex items-center gap-2.5 rounded-lg py-1.5 pr-1 pl-2.5 hover:bg-accent">
    {#if linker.picture}
      <img src={linker.picture} alt="" referrerpolicy="no-referrer" class="size-5 shrink-0 rounded-full object-cover" />
    {:else}
      <span class="flex size-5 shrink-0 items-center justify-center rounded-full bg-primary text-2xs font-semibold text-primary-foreground">
        {linker.name.slice(0, 1).toUpperCase()}
      </span>
    {/if}
    <span class="min-w-0 flex-1" title={linker.email ?? undefined}>
      <span class="block truncate text-xs font-medium">{linker.name}</span>
      <span class="block truncate text-2xs text-muted-foreground">
        {linker.plan ? `Linker ${linker.plan}` : "Linker"}{linker.email ? ` · ${linker.email}` : ""}
      </span>
    </span>
    <div class="hidden group-hover:block has-[[aria-expanded=true]]:block">
      <RowMenu
        label="Linker account"
        items={[
          { label: "Open Linker", Icon: ExternalLink, onSelect: () => window.open(linker.appUrl, "_blank", "noreferrer") },
          { label: "Sign out", Icon: LogOut, danger: true, onSelect: onSignOut },
        ]}
      />
    </div>
  </div>
{:else if linker.state === "signing_in"}
  <div class="mx-2 rounded-lg px-2.5 py-2 shadow-[0_0_0_1px_var(--border)]">
    <span class="block text-xs font-medium">Finish signing in in your browser</span>
    <span class="mt-0.5 flex gap-3 text-2xs">
      <a class="text-primary hover:underline" href={SIGN_IN} target="_blank" rel="noreferrer">Open it again</a>
      <button type="button" class="text-muted-foreground hover:text-foreground" onclick={onCancel}>Cancel</button>
    </span>
  </div>
{:else}
  {#if onThisComputer}
    <a
      class="mx-2 flex items-center gap-2.5 rounded-lg px-2.5 py-1.5 text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      href={SIGN_IN}
      target="_blank"
      rel="noreferrer"
      title="Optional. Brings your Linker account, and later its projects, to Linker-Edit."
    >
      <LogIn class="size-3.5 shrink-0" />
      <span class="text-xs">Sign in with Linker</span>
    </a>
  {:else}
    <span class="mx-2 flex items-center gap-2.5 px-2.5 py-1.5 text-2xs text-muted-foreground">
      <LogIn class="size-3.5 shrink-0" />
      Sign in with Linker on {computer}
    </span>
  {/if}
  {#if linker.problem}
    <span class="mx-2 block px-2.5 pb-1 text-2xs text-destructive">{linker.problem}</span>
  {/if}
{/if}
