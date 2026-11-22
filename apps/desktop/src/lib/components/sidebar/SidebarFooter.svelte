<script lang="ts">
  // The sidebar's foot: Linker-Edit's own parts, in T3 Code's SidebarFooter spacing (MIT, see NOTICE).
  import AudioLines from "@lucide/svelte/icons/audio-lines";
  import LogOut from "@lucide/svelte/icons/log-out";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import Settings from "@lucide/svelte/icons/settings";
  import type { AppMeta, ClientMessage } from "#shared/protocol.ts";
  import { useHost } from "#lib/hostContext.ts";
  import { resolveStatusLine } from "#lib/settings.ts";
  import AppUpdateCard from "../AppUpdateCard.svelte";
  import LinkerAccountRow from "../LinkerAccount.svelte";

  let {
    app,
    machine,
    asked,
    send,
    onSettings,
    onSignOut,
    onCheckAppUpdate,
  }: {
    /** The open machine's; null while none is connected. */
    app: AppMeta | null;
    /** The open machine's name, when there's more than one. */
    machine: string | null;
    /** The user just asked to check for updates, so the card also says when there's nothing new. */
    asked: boolean;
    /** To the open machine. */
    send: (message: ClientMessage) => void;
    onSettings: () => void;
    /** On a web address: signs this browser out of Linker. */
    onSignOut: (() => void) | null;
    onCheckAppUpdate: () => void;
  } = $props();
  const host = useHost();

  const remotePage = $derived(!host().local);
  const status = $derived(app ? resolveStatusLine(app.resolve) : null);
  // A computer that only drives an edit rig: nothing to set up here, so nothing to show.
  const controlOnly = $derived(!app || (!app.resolveHere && app.resolve.state === "setup_needed"));

  const transcribing = $derived.by(() => {
    if (!app) return null;
    const transcription = app.transcription;
    switch (transcription.state) {
      case "idle":
        if (!app.resolveHere) return null;
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
          onclick={() => send({ type: transcribing.action === "setup" ? "transcription_setup" : "transcription_cancel" })}
        >
          {transcribing.action === "setup" ? "Set up" : "Cancel"}
        </button>
      {/if}
    </div>
  {/if}

  {#if app?.appUpdate}
    <div class="-mx-2">
      <AppUpdateCard
        update={app.appUpdate}
        computer={app.computer}
        {remotePage}
        {asked}
        onInstall={() => send({ type: "install_app_update" })}
        onCancel={() => send({ type: "cancel_app_update" })}
        onCheck={onCheckAppUpdate}
      />
    </div>
  {:else if app?.update}
    <div class="rounded-lg px-2.5 py-2 shadow-[0_0_0_1px_var(--border)]">
      <span class="block text-xs font-medium">Linker-Edit {app.update} is out</span>
      <span class="mt-0.5 block text-2xs text-muted-foreground">
        Stop this one, then run <code class="font-mono">npx linker-edit@latest</code>.
      </span>
    </div>
  {/if}

  {#if app}
    <div class="-mx-2">
      <LinkerAccountRow
        linker={app.linker}
        computer={app.computer}
        onCancel={() => send({ type: "linker_cancel_sign_in" })}
        onSignOut={() => send({ type: "linker_sign_out" })}
      />
    </div>
  {/if}

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
    {#if onSignOut}
      <button
        type="button"
        class="flex size-8 shrink-0 cursor-pointer items-center justify-center rounded-(--control-radius) text-sidebar-muted-foreground/80 outline-hidden ring-ring hover:bg-sidebar-row-hover hover:text-sidebar-foreground focus-visible:ring-2 focus-visible:ring-inset [&>svg]:size-4 [&>svg]:text-(--sidebar-icon-color) hover:[&>svg]:text-sidebar-foreground"
        aria-label="Sign out"
        title="Sign out of Linker in this browser"
        onclick={onSignOut}
      >
        <LogOut />
      </button>
    {/if}
    {#if app && status && !controlOnly}
      <div class="min-w-0 flex-1">
        <button
          type="button"
          class="group flex min-h-8 w-full cursor-pointer items-start gap-2 rounded-(--control-radius) px-2 py-1.5 text-left hover:bg-sidebar-row-hover"
          onclick={() => send({ type: "refresh_resolve" })}
          title={[machine && `Resolve on ${machine}`, status.title, status.detail, "Click to refresh"].filter(Boolean).join("\n")}
        >
          <span class="mt-1.5 size-2 shrink-0 rounded-full {status.tone}"></span>
          <span class="min-w-0 flex-1">
            {#if machine}
              <span class="block truncate text-2xs text-muted-foreground">{machine}</span>
            {/if}
            <span class="block truncate text-xs font-medium">{status.title}</span>
            {#if status.detail}
              <span class="mt-0.5 block text-2xs text-muted-foreground">{status.detail}</span>
            {/if}
          </span>
          <RefreshCw class="mt-0.5 size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
        </button>
        {#if app.resolve.state === "setup_needed"}
          <button
            type="button"
            class="mb-1 ml-6 text-2xs text-primary hover:underline"
            title="For a computer that only controls your edit computer. Comes back if Resolve is installed there."
            onclick={() => send({ type: "set_resolve_here", resolveHere: false })}
          >
            Resolve won't be on {machine ?? "this computer"}
          </button>
        {/if}
      </div>
    {/if}
  </div>
</div>
