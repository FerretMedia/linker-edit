<script lang="ts">
  import { onMount } from "svelte";
  import Check from "@lucide/svelte/icons/check";
  import Copy from "@lucide/svelte/icons/copy";
  import Monitor from "@lucide/svelte/icons/monitor";
  import X from "@lucide/svelte/icons/x";
  import type { LinkerConnect } from "#shared/protocol.ts";
  import { host } from "#lib/host.ts";
  import SettingsGroup from "./SettingsGroup.svelte";
  import SettingsRow from "./SettingsRow.svelte";
  import Switch from "./Switch.svelte";

  // Linker Connect, like T3 Connect: this computer gets its own address at Linker, and your other computers
  // signed in to the same Linker account open it from anywhere. One account links up to connect.limit computers.
  let {
    connect,
    computer,
    onRefresh,
    onSetEnabled,
    onSetTunnel,
    onUnlink,
    onSignOutDevice,
    onSignIn,
  }: {
    connect: LinkerConnect;
    /** This computer's name. */
    computer: string;
    onRefresh: () => void;
    onSetEnabled: (enabled: boolean) => void;
    /** "Use the tunnel": off links this computer for Tailscale only. */
    onSetTunnel: (tunnel: boolean) => void;
    onUnlink: (computerId: string) => void;
    onSignOutDevice: (deviceId: string) => void;
    /** Opens the Linker account settings, to sign in. */
    onSignIn: () => void;
  } = $props();

  onMount(() => onRefresh());

  const button = "h-8 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
  // Opened through this computer's own tunnel, turning it off or unlinking it cuts this window off.
  const throughTunnel = $derived(connect.url !== null && host.base === new URL(connect.url).origin);
  const linkedHere = $derived(connect.computers?.some((entry) => entry.id === connect.computerId) ?? false);
  const count = $derived(connect.computers?.length ?? 0);

  let copied = $state(false);
  async function copy(text: string) {
    await navigator.clipboard.writeText(text);
    copied = true;
    setTimeout(() => (copied = false), 1500);
  }

  function setEnabled(enabled: boolean) {
    if (!enabled && throughTunnel && !confirm("You're connected through Linker Connect. Turning it off disconnects this window.")) return;
    onSetEnabled(enabled);
  }

  function setTunnel(tunnel: boolean) {
    if (!tunnel && throughTunnel && !confirm("You're connected through the tunnel. Turning it off disconnects this window.")) return;
    onSetTunnel(tunnel);
  }

  // Linked for Tailscale only: on, with no tunnel to connect.
  const ready = $derived(connect.state === "on" && (connect.connected || !connect.tunnel));

  function unlink(id: string, name: string) {
    const here = id === connect.computerId;
    const text = here
      ? `Unlink ${name}? It loses its address at Linker, and every device signed in to it is signed out.`
      : `Unlink ${name}? Its tunnel stops working until you turn Linker Connect on there again.`;
    if (confirm(here && throughTunnel ? `${text} This window is disconnected too.` : text)) onUnlink(id);
  }

  const when = (iso: string | null) => (iso ? new Date(iso).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" }) : "never");
</script>

{#if connect.state === "signed_out"}
  <SettingsGroup title="Linker Connect" description="Open this computer from your other computers, from anywhere. It needs your Linker account.">
    <SettingsRow label="Sign in with Linker first" description="Then sign in with the same account on the computers you work from.">
      <button type="button" class={button} onclick={onSignIn}>Linker account</button>
    </SettingsRow>
  </SettingsGroup>
{:else}
  <SettingsGroup
    title="Linker Connect"
    description="Open this computer from your other computers, from anywhere: they sign in with the same Linker account. Resolve and the agents keep running here."
  >
    <SettingsRow
      label="Linker Connect on {computer}"
      description={connect.state === "starting"
        ? (connect.step ?? "Starting…")
        : connect.state === "on"
          ? ready
            ? connect.tunnel
              ? "On. Reachable while Linker-Edit runs here."
              : "On, over Tailscale only. Turn on remote access below."
            : "Connecting to Cloudflare…"
          : `Gives ${computer} its own address at Linker. One account links up to ${connect.limit} computers.`}
    >
      {#snippet detail()}
        {#if connect.problem}
          <p class="mt-1 text-xs text-destructive">{connect.problem}</p>
        {/if}
      {/snippet}
      <span
        class="size-2 rounded-full {ready
          ? 'bg-success'
          : connect.state === 'off'
            ? 'bg-muted-foreground'
            : 'bg-warning'}"
      ></span>
      <Switch
        label="Linker Connect"
        checked={connect.state === "on" || connect.state === "starting"}
        onChange={(checked) => connect.state !== "starting" && setEnabled(checked)}
      />
    </SettingsRow>
    <SettingsRow
      label="Use the tunnel"
      description="Gives {computer} an address at Linker that works from anywhere. Off, your other devices reach it over Tailscale only."
    >
      <Switch label="Use the tunnel" checked={connect.tunnel} onChange={(checked) => connect.state !== "starting" && setTunnel(checked)} />
    </SettingsRow>
    {#if connect.url && connect.state === "on"}
      <SettingsRow label="Address" description="On your other computer, pick {computer} in Linker-Edit's computer menu.">
        {#snippet detail()}
          <p class="mt-1 font-mono text-xs break-all select-all">{connect.url}</p>
        {/snippet}
        <button type="button" class="{button} flex items-center gap-1.5" onclick={() => copy(connect.url!)}>
          {#if copied}<Check class="size-3.5" />{:else}<Copy class="size-3.5" />{/if}
          Copy
        </button>
      </SettingsRow>
    {/if}
  </SettingsGroup>

  <SettingsGroup
    title="Your linked computers{connect.computers ? ` · ${count} of ${connect.limit}` : ''}"
    description="The edit computers linked to your Linker account. Computers you only work from don't count."
  >
    {#if connect.computers === null}
      <SettingsRow label="Looking them up…" />
    {:else if connect.computers.length === 0}
      <SettingsRow label="None yet" description="Turn Linker Connect on to link {computer}." />
    {:else}
      {#each connect.computers as entry (entry.id)}
        <SettingsRow
          label="{entry.name}{entry.id === connect.computerId ? ' (this computer)' : ''}"
          description="{entry.online === true ? 'Online' : entry.online === false ? 'Offline' : 'Not known'} · linked {when(entry.linkedAt)}"
        >
          <Monitor class="size-3.5 text-muted-foreground" />
          <button type="button" class={button} onclick={() => unlink(entry.id, entry.name)}>Unlink</button>
        </SettingsRow>
      {/each}
    {/if}
    {#if connect.atLimit && !linkedHere}
      <SettingsRow label="Unlink one to link {computer}" description="Each Linker account links up to {connect.limit} computers.">
        <button type="button" class={button} onclick={() => setEnabled(true)}>Try again</button>
      </SettingsRow>
    {/if}
  </SettingsGroup>

  <SettingsGroup
    title="Signed-in devices"
    description="Computers and the iPad app that opened {computer} through Linker Connect. Signing one out cuts it off at once."
  >
    {#if connect.devices.length === 0}
      <SettingsRow label="None" />
    {:else}
      {#each connect.devices as device (device.id)}
        <SettingsRow
          label={device.name}
          description="{device.kind === 'native' ? 'Native app · ' : ''}Signed in {when(device.createdAt)} · last used {when(device.lastSeenAt)}"
        >
          <button type="button" class="{button} flex items-center gap-1.5" onclick={() => onSignOutDevice(device.id)}>
            <X class="size-3.5" />
            Sign out
          </button>
        </SettingsRow>
      {/each}
    {/if}
  </SettingsGroup>
{/if}
