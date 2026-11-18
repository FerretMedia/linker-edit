<script lang="ts">
  // The sidebar's foot: Linker-Edit's own parts, in T3 Code's SidebarFooter spacing (MIT, see NOTICE).
  import AudioLines from "@lucide/svelte/icons/audio-lines";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import Settings from "@lucide/svelte/icons/settings";
  import type { AppUpdate, LinkerAccount, ResolveStatus, TranscriptionStatus } from "#shared/protocol.ts";
  import { resolveStatusLine } from "#lib/settings.ts";
  import AppUpdateCard from "../AppUpdateCard.svelte";
  import LinkerAccountRow from "../LinkerAccount.svelte";

  let {
    resolve,
    resolveHere,
    update,
    appUpdate,
    transcription,
    computer,
    linker,
    asked,
    onRefreshResolve,
    onResolveHere,
    onSettings,
    onLinkerCancel,
    onLinkerSignOut,
    onTranscriptionSetup,
    onTranscriptionCancel,
    onCheckAppUpdate,
    onInstallAppUpdate,
    onCancelAppUpdate,
  }: {
    resolve: ResolveStatus;
    resolveHere: boolean;
    update: string | null;
    appUpdate: AppUpdate | null;
    transcription: TranscriptionStatus;
    computer: string;
    linker: LinkerAccount;
    /** The user just asked to check for updates, so the card also says when there's nothing new. */
    asked: boolean;
    onRefreshResolve: () => void;
    onResolveHere: (resolveHere: boolean) => void;
    onSettings: () => void;
    onLinkerCancel: () => void;
    onLinkerSignOut: () => void;
    onTranscriptionSetup: () => void;
    onTranscriptionCancel: () => void;
    onCheckAppUpdate: () => void;
    onInstallAppUpdate: () => void;
    onCancelAppUpdate: () => void;
  } = $props();

  const remotePage = !["127.0.0.1", "localhost", "[::1]"].includes(location.hostname);
  const status = $derived(resolveStatusLine(resolve));
  // A computer that only drives an edit rig: nothing to set up here, so nothing to show.
  const controlOnly = $derived(!resolveHere && resolve.state === "setup_needed");

  const transcribing = $derived.by(() => {
    switch (transcription.state) {
      case "idle":
        if (!resolveHere) return null;
        // Downloaded by itself when the app starts; this is the way to try again after that failed.
        return transcription.ready === false
          ? { title: "Transcription isn't set up", detail: "A one-time download of up to 1.6 GB", action: "setup" as const }
          : null;
      case "preparing":
        return {
          title: transcription.percent === null ? transcription.step : `${transcription.step} · ${transcription.percent}%`,
          detail: "Only the first time",
          action: "cancel" as const,
        };
      case "transcribing":
        return {
          title: "Transcribing",
          detail: transcription.total > 1 ? `${transcription.file} (${transcription.done + 1} of ${transcription.total})` : transcription.file,
        };
    }
  });
</script>

<div class="flex flex-col gap-1 px-(--sidebar-content-inset) pt-1 pb-(--sidebar-content-inset)">
  {#if transcribing}
    <div class="flex items-start gap-2.5 px-2.5 py-2" title={transcribing.detail}>
      <AudioLines class="mt-0.5 size-3.5 shrink-0 text-primary" />
      <span class="min-w-0 flex-1">
        <span class="block truncate text-xs font-medium">{transcribing.title}</span>
        <span class="mt-0.5 block truncate text-2xs text-muted-foreground">{transcribing.detail}</span>
      </span>
      {#if transcribing.action}
        <button
          type="button"
          class="shrink-0 text-2xs text-primary hover:underline"
          onclick={transcribing.action === "setup" ? onTranscriptionSetup : onTranscriptionCancel}
        >
          {transcribing.action === "setup" ? "Set up" : "Cancel"}
        </button>
      {/if}
    </div>
  {/if}

  {#if appUpdate}
    <div class="-mx-2">
      <AppUpdateCard update={appUpdate} {computer} {remotePage} {asked} onInstall={onInstallAppUpdate} onCancel={onCancelAppUpdate} onCheck={onCheckAppUpdate} />
    </div>
  {:else if update}
    <div class="rounded-lg px-2.5 py-2 shadow-[0_0_0_1px_var(--border)]">
      <span class="block text-xs font-medium">Linker-Edit {update} is out</span>
      <span class="mt-0.5 block text-2xs text-muted-foreground">
        Stop this one, then run <code class="font-mono">npx linker-edit@latest</code>.
      </span>
    </div>
  {/if}

  <div class="-mx-2">
    <LinkerAccountRow {linker} {computer} onCancel={onLinkerCancel} onSignOut={onLinkerSignOut} />
  </div>

  <div class="flex items-start gap-1">
    <button
      type="button"
      class="flex size-8 shrink-0 cursor-pointer items-center justify-center rounded-(--control-radius) text-sidebar-muted-foreground/80 outline-hidden ring-ring hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-inset [&>svg]:size-4 [&>svg]:text-(--sidebar-icon-color) hover:[&>svg]:text-sidebar-foreground"
      aria-label="Settings"
      title="Settings (Ctrl+,)"
      onclick={onSettings}
    >
      <Settings />
    </button>
    {#if !controlOnly}
      <div class="min-w-0 flex-1">
        <button
          type="button"
          class="group flex min-h-8 w-full cursor-pointer items-start gap-2 rounded-(--control-radius) px-2 py-1.5 text-left hover:bg-sidebar-row-hover"
          onclick={onRefreshResolve}
          title={status.detail ? `${status.title}\n${status.detail}\nClick to refresh` : `${status.title}\nClick to refresh`}
        >
          <span class="mt-1.5 size-2 shrink-0 rounded-full {status.tone}"></span>
          <span class="min-w-0 flex-1">
            <span class="block truncate text-xs font-medium">{status.title}</span>
            {#if status.detail}
              <span class="mt-0.5 block text-2xs text-muted-foreground">{status.detail}</span>
            {/if}
          </span>
          <RefreshCw class="mt-0.5 size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
        </button>
        {#if resolve.state === "setup_needed"}
          <button
            type="button"
            class="mb-1 ml-6 text-2xs text-primary hover:underline"
            title="For a computer that only controls your edit computer. Comes back if Resolve is installed here."
            onclick={() => onResolveHere(false)}
          >
            Resolve won't be on this computer
          </button>
        {/if}
      </div>
    {/if}
  </div>
</div>
