<script lang="ts">
  import ArrowLeft from "@lucide/svelte/icons/arrow-left";
  import AudioLines from "@lucide/svelte/icons/audio-lines";
  import Check from "@lucide/svelte/icons/check";
  import Clapperboard from "@lucide/svelte/icons/clapperboard";
  import Copy from "@lucide/svelte/icons/copy";
  import ExternalLink from "@lucide/svelte/icons/external-link";
  import Globe from "@lucide/svelte/icons/globe";
  import Info from "@lucide/svelte/icons/info";
  import Library from "@lucide/svelte/icons/library";
  import LogIn from "@lucide/svelte/icons/log-in";
  import LogOut from "@lucide/svelte/icons/log-out";
  import Monitor from "@lucide/svelte/icons/monitor";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import Settings2 from "@lucide/svelte/icons/settings-2";
  import UserRound from "@lucide/svelte/icons/user-round";
  import { AUTO_SETTLE_DAYS, type AppMeta, type ClientMessage } from "#shared/protocol.ts";
  import { RELEASES_URL, releaseName } from "#shared/release-name.ts";
  import type { Host } from "#lib/host.ts";
  import { provideHost } from "#lib/hostContext.ts";
  import { PROVIDER_LABEL } from "#lib/labels.ts";
  import { resolveStatusLine, SETTINGS_SECTIONS, type SettingsSection } from "#lib/settings.ts";
  import { sidebarPrefs } from "#lib/sidebarPrefs.svelte.ts";
  import { loadTheme, saveTheme, type Theme } from "#lib/theme.ts";
  import LinkerConnectSettings from "./LinkerConnectSettings.svelte";
  import RemoteAccessSettings from "./RemoteAccessSettings.svelte";
  import Segmented from "./Segmented.svelte";
  import SettingsGroup from "./SettingsGroup.svelte";
  import SettingsRow from "./SettingsRow.svelte";
  import Switch from "./Switch.svelte";

  let {
    app,
    host,
    machines,
    machineId,
    onMachine,
    section,
    onSection,
    onLibrary,
    onSignOut = null,
    send,
    onClose,
  }: {
    /** The settings of this machine: each of the user's computers has its own. */
    app: AppMeta;
    host: Host;
    /** Every machine, to choose whose settings these are when there's more than one. */
    machines: { id: string; name: string; status: "connecting" | "connected" | "offline" }[];
    machineId: string;
    onMachine: (machineId: string) => void;
    section: SettingsSection;
    onSection: (section: SettingsSection) => void;
    /** Opens the library page, which has its own address. */
    onLibrary?: () => void;
    /** On a web address: signs this browser out of Linker. */
    onSignOut?: (() => void) | null;
    /** To this machine. */
    send: (message: ClientMessage) => void;
    onClose: () => void;
  } = $props();

  const ICONS = { general: Settings2, resolve: Clapperboard, transcription: AudioLines, remote: Globe, account: UserRound, about: Info };
  const title = $derived(SETTINGS_SECTIONS.find((entry) => entry.id === section)?.label ?? "Settings");

  // Its remote access and Linker account settings below talk to it too.
  provideHost(() => host);
  // Linker and Google send the browser back to 127.0.0.1, so signing in only works on the computer Linker-Edit runs on.
  const onThisComputer = $derived(host.local);
  const SIGN_IN = $derived(host.url("/api/linker/sign-in"));
  const GOOGLE_SIGN_IN = $derived(host.url("/api/google/sign-in"));

  let theme = $state<Theme>(loadTheme());
  function pickTheme(next: Theme) {
    theme = next;
    saveTheme(next);
  }

  const status = $derived(resolveStatusLine(app.resolve));

  let copied = $state<string | null>(null);
  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    copied = text;
    setTimeout(() => (copied = copied === text ? null : copied), 1500);
  }

  // Checking says how it went, even when there's nothing new.
  const updateLine = $derived.by((): { text: string; tone?: "error" } => {
    const update = app.appUpdate;
    if (!update) {
      return app.update
        ? { text: `Linker-Edit ${releaseName(app.update)} is out. Stop this one, then run npx linker-edit@latest.` }
        : { text: "Started with npx: a new version shows here when it's out." };
    }
    const version = releaseName(update.downloadedVersion ?? update.availableVersion ?? "");
    switch (update.status) {
      case "checking":
        return { text: "Looking for updates…" };
      case "available":
        return update.downloadUrl ? { text: `Linker-Edit ${version} is out.` } : { text: `Downloading Linker-Edit ${version}…` };
      case "downloading":
        return { text: `Downloading Linker-Edit ${version}${update.percent !== null ? ` · ${update.percent}%` : "…"}` };
      case "downloaded":
        return update.waiting
          ? { text: `Linker-Edit restarts into ${version} once Resolve is free.` }
          : { text: `Linker-Edit ${version} is ready. It installs when you restart or quit.` };
      case "error":
        return { text: update.error ?? "Couldn't look for updates.", tone: "error" };
      default:
        return {
          text: update.checkedAt
            ? `Up to date. Last checked ${new Date(update.checkedAt).toLocaleString([], { dateStyle: "medium", timeStyle: "short" })}.`
            : "Looks for updates a few seconds after starting, then every hour.",
        };
    }
  });

  const transcription = $derived.by((): { text: string; percent?: number | null; action?: "setup" | "cancel" | "ready"; actionLabel?: string } => {
    const state = app.transcription;
    switch (state.state) {
      case "preparing":
        return { text: state.step, percent: state.percent, action: "cancel" };
      case "transcribing":
        return {
          text: `Transcribing ${state.file}${state.total > 1 ? ` (${state.done + 1} of ${state.total})` : ""}`,
          action: "ready",
        };
      case "idle":
        if (state.ready) return { text: "Installed. Transcripts are made on this computer; nothing is uploaded.", action: "ready" };
        if (!app.resolveHere) {
          return {
            text: "Not downloaded here, because Resolve won't be on this computer.",
            action: "setup",
            actionLabel: "Download anyway",
          };
        }
        if (state.ready === false) {
          return {
            text: "Not downloaded yet. It downloads by itself when Linker-Edit starts: up to 1.6 GB, only once.",
            action: "setup",
            actionLabel: "Download now",
          };
        }
        return { text: "Downloads with the first transcription." };
    }
  });

  const folders = $derived(
    app.setup
      ? [
          { label: "Projects", path: app.setup.folders.projects },
          { label: "Preview renders", path: app.setup.folders.previews },
          { label: "Transcripts", path: app.setup.folders.transcripts },
          { label: "Speech engine", path: app.setup.folders.tools },
          { label: "Config file", path: app.setup.configFile },
        ]
      : [],
  );

  const button = "h-8 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
  const primaryButton = "h-8 rounded-md bg-primary px-3 text-xs whitespace-nowrap text-primary-foreground transition-opacity hover:opacity-90";
</script>

{#snippet path(text: string)}
  <span class="mt-1 flex min-w-0 items-center gap-1.5">
    <code class="min-w-0 truncate font-mono text-2xs text-muted-foreground" title={text}>{text}</code>
    <button
      type="button"
      class="flex size-5 shrink-0 items-center justify-center rounded text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      aria-label="Copy path"
      title="Copy path"
      onclick={() => copy(text)}
    >
      {#if copied === text}<Check class="size-3" />{:else}<Copy class="size-3" />{/if}
    </button>
  </span>
{/snippet}

<div class="title-bar-row flex h-full flex-col md:flex-row">
  <nav class="flex shrink-0 flex-col border-b bg-sidebar md:w-60 md:border-r md:border-b-0" aria-label="Settings">
    <div class="title-bar title-bar-start flex h-13 shrink-0 items-center px-2">
      <button
        type="button"
        class="flex h-8 items-center gap-2 rounded-lg px-2.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
        title="Back (Esc)"
        onclick={onClose}
      >
        <ArrowLeft class="size-4" />
        Back
      </button>
    </div>
    {#if machines.length > 1}
      <!-- Like T3 Code's environments: each machine has its own settings. -->
      <ul class="flex gap-1 overflow-x-auto border-b px-2 pb-2 md:mb-2 md:flex-col md:overflow-visible" aria-label="Computers">
        {#each machines as entry (entry.id)}
          <li class="shrink-0">
            <button
              type="button"
              class="flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm whitespace-nowrap transition-colors disabled:opacity-50 {entry.id ===
              machineId
                ? 'bg-sidebar-row-active font-medium text-foreground shadow-[0_0_0_1px_var(--border)]'
                : 'text-muted-foreground hover:bg-accent hover:text-foreground disabled:hover:bg-transparent'}"
              aria-current={entry.id === machineId ? "true" : undefined}
              disabled={entry.status !== "connected"}
              title={entry.status === "connected" ? `Settings of Linker-Edit on ${entry.name}` : `${entry.name} is ${entry.status}`}
              onclick={() => onMachine(entry.id)}
            >
              <Monitor class="size-4 shrink-0" />
              <span class="min-w-0 flex-1 truncate">{entry.name}</span>
              {#if entry.status !== "connected"}
                <span class="shrink-0 text-2xs">{entry.status}</span>
              {/if}
            </button>
          </li>
        {/each}
      </ul>
    {/if}
    <ul class="flex gap-1 overflow-x-auto px-2 pb-2 md:flex-col md:overflow-visible">
      {#each SETTINGS_SECTIONS as entry (entry.id)}
        {@const Icon = ICONS[entry.id]}
        <li class="shrink-0">
          <button
            type="button"
            class="flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm whitespace-nowrap transition-colors {entry.id === section
              ? 'bg-sidebar-row-active font-medium text-foreground shadow-[0_0_0_1px_var(--border)]'
              : 'text-muted-foreground hover:bg-accent hover:text-foreground'}"
            aria-current={entry.id === section ? "page" : undefined}
            onclick={() => onSection(entry.id)}
          >
            <Icon class="size-4 shrink-0" />
            {entry.label}
          </button>
        </li>
      {/each}
      {#if onLibrary}
        <li class="shrink-0 md:mt-2 md:border-t md:pt-2">
          <button
            type="button"
            class="flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm whitespace-nowrap text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            onclick={onLibrary}
          >
            <Library class="size-4 shrink-0" />
            Library
          </button>
        </li>
      {/if}
      {#if onSignOut}
        <li class="shrink-0 md:mt-2 md:border-t md:pt-2">
          <button
            type="button"
            class="flex h-8 w-full items-center gap-2.5 rounded-lg px-2.5 text-left text-sm whitespace-nowrap text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
            title="Signs this browser out of Linker; your computers keep running"
            onclick={onSignOut}
          >
            <LogOut class="size-4 shrink-0" />
            Sign out
          </button>
        </li>
      {/if}
    </ul>
    <p class="mt-auto hidden px-4 py-3 text-2xs text-muted-foreground md:block">
      Settings of Linker-Edit on <span class="text-foreground">{app.computer}</span>
    </p>
  </nav>

  <main class="flex min-h-0 min-w-0 flex-1 flex-col">
    <header class="title-bar flex h-13 shrink-0 items-center border-b px-5 md:px-8">
      <h1 class="text-sm font-medium">{title}</h1>
    </header>

    <div class="min-h-0 flex-1 overflow-y-auto">
      <div class="mx-auto flex w-full max-w-2xl flex-col gap-8 px-4 py-6 md:px-8 md:py-8">
        {#if section === "general"}
          <SettingsGroup title="Agents">
            <SettingsRow
              label="Approvals"
              description={app.approvalMode === "auto"
                ? "Full access: changes run without asking. The project is backed up first, and agents only change their own AI timelines."
                : "Supervised: you Allow or Deny each change to Resolve, and Codex stays in the project's folder."}
            >
              <Segmented
                label="Approvals"
                options={[
                  { value: "auto", label: "Full access" },
                  { value: "ask", label: "Supervised" },
                ]}
                value={app.approvalMode}
                onChange={(mode) => send({ type: "set_approval_mode", mode })}
              />
            </SettingsRow>
            <SettingsRow label="New threads start with" description="You can switch agents in any thread, even mid-chat.">
              <Segmented
                label="New threads start with"
                options={[
                  { value: "claude", label: PROVIDER_LABEL.claude },
                  { value: "codex", label: PROVIDER_LABEL.codex },
                ]}
                value={app.defaultProvider}
                onChange={(provider) => send({ type: "set_default_provider", provider })}
              />
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup title="Appearance">
            <SettingsRow label="Theme" description="Kept per window, so each of your computers can have its own.">
              <Segmented
                label="Theme"
                options={[
                  { value: "system", label: "System" },
                  { value: "light", label: "Light" },
                  { value: "dark", label: "Dark" },
                ]}
                value={theme}
                onChange={pickTheme}
              />
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup title="Sidebar">
            <SettingsRow
              label="Group threads into project folders"
              description="A folder per project instead of one list of every project's threads. Pinned threads stay on top, Snoozed and Settled below the folders, and running threads stay in their folder. Kept per window."
            >
              <Switch label="Group threads into project folders" checked={sidebarPrefs.folders} onChange={(folders) => sidebarPrefs.setFolders(folders)} />
            </SettingsRow>
            <!-- Older Linker-Edits don't settle threads by themselves. -->
            {#if app.autoSettleAfterDays !== undefined}
              <SettingsRow
                label="Settle quiet threads"
                description="Threads with no activity for this long move to Settled by themselves. Writing in one brings it back. Each computer has its own."
              >
                <Switch
                  label="Settle quiet threads"
                  checked={app.autoSettleAfterDays !== null}
                  onChange={(on) => send({ type: "set_auto_settle", days: on ? AUTO_SETTLE_DAYS.default : null })}
                />
              </SettingsRow>
              {#if app.autoSettleAfterDays !== null}
                <SettingsRow label="Days without activity" description="From {AUTO_SETTLE_DAYS.min} to {AUTO_SETTLE_DAYS.max}.">
                  <input
                    type="number"
                    min={AUTO_SETTLE_DAYS.min}
                    max={AUTO_SETTLE_DAYS.max}
                    value={app.autoSettleAfterDays}
                    aria-label="Days without activity before a thread settles"
                    class="h-8 w-20 rounded-md border bg-transparent px-2.5 text-sm tabular-nums outline-none focus:border-primary"
                    onchange={(event) => {
                      const days = Number(event.currentTarget.value);
                      if (Number.isInteger(days) && days >= AUTO_SETTLE_DAYS.min && days <= AUTO_SETTLE_DAYS.max) send({ type: "set_auto_settle", days });
                      else event.currentTarget.value = String(app.autoSettleAfterDays);
                    }}
                  />
                </SettingsRow>
              {/if}
            {/if}
          </SettingsGroup>
        {:else if section === "resolve"}
          <SettingsGroup title="Status">
            <SettingsRow label={status.title}>
              {#snippet detail()}
                <p class="mt-0.5 flex items-start gap-2 text-xs text-muted-foreground">
                  <span class="mt-1 size-2 shrink-0 rounded-full {status.tone}"></span>
                  <span>{status.detail || "Checked every 15 seconds while Linker-Edit is idle."}</span>
                </p>
              {/snippet}
              <button type="button" class="{button} flex items-center gap-1.5" onclick={() => send({ type: "refresh_resolve" })}>
                <RefreshCw class="size-3.5" />
                Refresh
              </button>
            </SettingsRow>
          </SettingsGroup>

          <SettingsGroup title={app.computer} description="DaVinci Resolve Studio 21.1.2 or newer, with External scripting set to Local.">
            <SettingsRow
              label="Resolve is on this computer"
              description="Turn off on a computer that only controls your edit computer. Resolve's setup and the speech engine are then left out there. Turn it back on to set Resolve up there; it also turns on by itself when Resolve is installed."
            >
              <Switch
                label="Resolve is on this computer"
                checked={app.resolveHere}
                onChange={(resolveHere) => send({ type: "set_resolve_here", resolveHere })}
              />
            </SettingsRow>
            {#if app.setup}
              <SettingsRow
                label="Resolve's MCP server"
                description={app.setup.resolveMcp
                  ? "How Linker-Edit drives Resolve. It comes with Resolve Studio."
                  : "Not found. Install Resolve Studio, or set resolveMcp in the config file to where it is, then restart Linker-Edit."}
              >
                {#snippet detail()}
                  {#if app.setup?.resolveMcp}{@render path(app.setup.resolveMcp)}{/if}
                {/snippet}
              </SettingsRow>
            {/if}
          </SettingsGroup>
        {:else if section === "transcription"}
          <SettingsGroup
            title="Speech engine"
            description="whisper.cpp with the large-v3-turbo model: most languages, with a time for every word, on this computer's GPU when it has one."
          >
            <SettingsRow label={transcription.action === "ready" ? "Ready" : "whisper.cpp"} description={transcription.text}>
              {#snippet detail()}
                {#if transcription.percent !== undefined}
                  <div class="mt-2 h-1 overflow-hidden rounded-full bg-border">
                    <div
                      class="h-full rounded-full bg-primary transition-[width] {transcription.percent === null ? 'w-1/3 animate-pulse' : ''}"
                      style:width={transcription.percent === null ? undefined : `${transcription.percent}%`}
                    ></div>
                  </div>
                  {#if transcription.percent !== null}
                    <p class="mt-1 text-2xs text-muted-foreground">{transcription.percent}%</p>
                  {/if}
                {/if}
              {/snippet}
              {#if transcription.action === "setup"}
                <button type="button" class={app.resolveHere ? primaryButton : button} onclick={() => send({ type: "transcription_setup" })}>
                  {transcription.actionLabel}
                </button>
              {:else if transcription.action === "cancel"}
                <button type="button" class={button} onclick={() => send({ type: "transcription_cancel" })}>Cancel</button>
              {:else if transcription.action === "ready"}
                <span class="flex items-center gap-1.5 text-xs text-success"><Check class="size-3.5" />Installed</span>
              {/if}
            </SettingsRow>
          </SettingsGroup>
          {#if app.setup}
            <SettingsGroup title="Where it's kept">
              <SettingsRow label="Transcripts" description="One text file and one JSON file with word times per clip, kept until the clip changes.">
                {#snippet detail()}{@render path(app.setup!.folders.transcripts)}{/snippet}
              </SettingsRow>
              <SettingsRow label="Speech engine and model">
                {#snippet detail()}{@render path(app.setup!.folders.tools)}{/snippet}
              </SettingsRow>
            </SettingsGroup>
          {/if}
        {:else if section === "remote"}
          <LinkerConnectSettings
            connect={app.connect}
            computer={app.computer}
            onRefresh={() => send({ type: "refresh_connect" })}
            onSetEnabled={(enabled) => send({ type: "set_connect", enabled })}
            onSetTunnel={(tunnel) => send({ type: "set_connect_tunnel", tunnel })}
            onUnlink={(computerId) => send({ type: "connect_unlink", computerId })}
            onSignOutDevice={(deviceId) => send({ type: "connect_sign_out_device", deviceId })}
            onSignIn={() => onSection("account")}
          />
          <section class="flex flex-col gap-2">
            <h2 class="px-1 text-sm font-medium">Tailscale</h2>
            <p class="px-1 text-xs text-muted-foreground">
              Or let your other computers use this one over
              <a class="text-primary underline underline-offset-2" href="https://tailscale.com" target="_blank" rel="noreferrer">Tailscale</a>.
              Resolve and the agents keep running here; only devices logged in to your Tailscale account get in.
            </p>
            <div class="rounded-xl border bg-card p-4">
              <RemoteAccessSettings
                remote={app.remote}
                onRefresh={() => send({ type: "refresh_remote_access" })}
                onSetEnabled={(enabled) => send({ type: "set_remote_access", enabled })}
              />
            </div>
          </section>
        {:else if section === "account"}
          <SettingsGroup title="Linker" description="Optional. Brings your Linker account, and later its projects, to Linker-Edit. Nothing else needs it.">
            {#if app.linker.state === "signed_in"}
              {@const linker = app.linker}
              <div class="flex flex-col gap-3 p-4 sm:flex-row sm:items-center">
                <div class="flex min-w-0 flex-1 items-center gap-3">
                  {#if linker.picture}
                    <img src={linker.picture} alt="" referrerpolicy="no-referrer" class="size-9 shrink-0 rounded-full object-cover" />
                  {:else}
                    <span class="flex size-9 shrink-0 items-center justify-center rounded-full bg-primary text-sm font-semibold text-primary-foreground">
                      {linker.name.slice(0, 1).toUpperCase()}
                    </span>
                  {/if}
                  <div class="min-w-0">
                    <p class="truncate text-sm font-medium">{linker.name}</p>
                    <p class="truncate text-xs text-muted-foreground">
                      {linker.plan ? `Linker ${linker.plan}` : "Linker"}{linker.email ? ` · ${linker.email}` : ""}
                    </p>
                  </div>
                </div>
                <div class="flex shrink-0 gap-2">
                  <a class="{button} flex items-center gap-1.5" href={linker.appUrl} target="_blank" rel="noreferrer">
                    <ExternalLink class="size-3.5" />
                    Open Linker
                  </a>
                  <button type="button" class="{button} flex items-center gap-1.5 text-destructive" onclick={() => send({ type: "linker_sign_out" })}>
                    <LogOut class="size-3.5" />
                    Sign out
                  </button>
                </div>
              </div>
            {:else if app.linker.state === "signing_in"}
              <SettingsRow label="Finish signing in in your browser" description="Linker-Edit is waiting for Linker to send you back.">
                <a class={button + " flex items-center"} href={SIGN_IN} target="_blank" rel="noreferrer">Open it again</a>
                <button type="button" class={button} onclick={() => send({ type: "linker_cancel_sign_in" })}>Cancel</button>
              </SettingsRow>
            {:else}
              <SettingsRow
                label="Not signed in"
                description={onThisComputer ? "Signing in opens Linker in your browser." : `Sign in on ${app.computer} itself: Linker sends your browser back to it.`}
              >
                {#snippet detail()}
                  {#if app.linker.state === "signed_out" && app.linker.problem}
                    <p class="mt-1 text-xs text-destructive">{app.linker.problem}</p>
                  {/if}
                {/snippet}
                {#if onThisComputer}
                  <a class="{primaryButton} flex items-center gap-1.5" href={SIGN_IN} target="_blank" rel="noreferrer">
                    <LogIn class="size-3.5" />
                    Sign in with Linker
                  </a>
                {/if}
              </SettingsRow>
            {/if}
          </SettingsGroup>

          {#if app.googleDrive}
            {@const drive = app.googleDrive}
            <SettingsGroup title="Google Drive" description="Where Linker-Edit will keep your projects for all your computers. It sees only the files it makes and the ones you pick.">
              {#if drive.state === "not_set_up"}
                <SettingsRow label="Not set up in this build" description="This Linker-Edit has no Google sign-in yet, so it can't use Google Drive." />
              {:else if drive.state === "signed_in"}
                <SettingsRow label={drive.email} description={drive.name ? `Signed in as ${drive.name}.` : "Signed in."}>
                  <button type="button" class="{button} flex items-center gap-1.5 text-destructive" onclick={() => send({ type: "google_sign_out" })}>
                    <LogOut class="size-3.5" />
                    Sign out
                  </button>
                </SettingsRow>
              {:else if drive.state === "signing_in"}
                <SettingsRow label="Finish signing in in your browser" description="Linker-Edit is waiting for Google to send you back.">
                  <a class={button + " flex items-center"} href={GOOGLE_SIGN_IN} target="_blank" rel="noreferrer">Open it again</a>
                  <button type="button" class={button} onclick={() => send({ type: "google_cancel_sign_in" })}>Cancel</button>
                </SettingsRow>
              {:else}
                <SettingsRow
                  label="Not signed in"
                  description={onThisComputer ? "Signing in opens Google in your browser." : `Sign in on ${app.computer} itself: Google sends your browser back to it.`}
                >
                  {#snippet detail()}
                    {#if drive.problem}
                      <p class="mt-1 text-xs text-destructive">{drive.problem}</p>
                    {/if}
                  {/snippet}
                  {#if onThisComputer}
                    <a class="{primaryButton} flex items-center gap-1.5" href={GOOGLE_SIGN_IN} target="_blank" rel="noreferrer">
                      <LogIn class="size-3.5" />
                      Sign in
                    </a>
                  {/if}
                </SettingsRow>
              {/if}
            </SettingsGroup>
          {/if}
        {:else if section === "about"}
          <SettingsGroup title="Linker-Edit">
            <SettingsRow label="Version {releaseName(app.version)}">
              {#snippet detail()}
                <p class="mt-0.5 text-xs {updateLine.tone === 'error' ? 'text-destructive' : 'text-muted-foreground'}">{updateLine.text}</p>
                {#if app.appUpdate?.status === "downloading" && app.appUpdate.percent !== null}
                  <div class="mt-2 h-1 overflow-hidden rounded-full bg-border">
                    <div class="h-full rounded-full bg-primary transition-[width]" style:width="{app.appUpdate.percent}%"></div>
                  </div>
                {/if}
                {#if app.appUpdate?.notes.length && (app.appUpdate.status === "downloaded" || app.appUpdate.status === "available")}
                  <ul class="mt-2 list-disc space-y-0.5 pl-4 text-xs text-muted-foreground">
                    {#each app.appUpdate.notes as note, index (index)}
                      <li>{note}</li>
                    {/each}
                  </ul>
                {/if}
              {/snippet}
              {#if app.appUpdate?.status === "downloaded" && !app.appUpdate.waiting}
                <button type="button" class={primaryButton} onclick={() => send({ type: "install_app_update" })}>
                  {onThisComputer ? "Restart to update" : `Restart ${app.computer} to update`}
                </button>
              {:else if app.appUpdate?.status === "downloaded"}
                <button type="button" class={button} onclick={() => send({ type: "cancel_app_update" })}>Not now</button>
              {:else if app.appUpdate?.status === "available" && app.appUpdate.downloadUrl}
                <a class="{primaryButton} flex items-center" href={app.appUpdate.downloadUrl} target="_blank" rel="noreferrer">Download it</a>
              {:else if app.appUpdate}
                <button
                  type="button"
                  class={button}
                  disabled={app.appUpdate.status === "checking" || app.appUpdate.status === "downloading"}
                  onclick={() => send({ type: "check_app_update" })}
                >
                  Check for updates
                </button>
              {/if}
            </SettingsRow>
            <SettingsRow label="Releases" description="What changed in each version, and the installers for your other computers.">
              <a class="{button} flex items-center gap-1.5" href={RELEASES_URL} target="_blank" rel="noreferrer">
                <ExternalLink class="size-3.5" />
                Open
              </a>
            </SettingsRow>
          </SettingsGroup>

          {#if folders.length}
            <SettingsGroup
              title="Files on {app.computer}"
              description="To keep them somewhere else, set them in the config file (see linker-edit.config.example.json), then restart Linker-Edit."
            >
              {#each folders as folder (folder.label)}
                <SettingsRow label={folder.label}>
                  {#snippet detail()}{@render path(folder.path)}{/snippet}
                </SettingsRow>
              {/each}
            </SettingsGroup>
          {/if}
        {/if}
      </div>
    </div>
  </main>
</div>
