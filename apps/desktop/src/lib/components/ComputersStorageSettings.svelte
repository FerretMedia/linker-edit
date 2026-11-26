<script lang="ts">
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import { untrack } from "svelte";
  import type { StorageCard } from "#shared/protocol.ts";
  import { Host } from "#lib/host.ts";
  import { machines, type MachineConnection } from "#lib/machines.svelte.ts";
  import { formatBytes, loadStorageCard } from "#lib/storage.ts";
  import SettingsGroup from "./SettingsGroup.svelte";

  // Every computer's disks and the space its projects take there, a group for each computer.
  const home = new Host();
  /** Each computer's last card, by machine id, kept on screen while it's measured again. */
  let cards = $state<Record<string, { card: StorageCard | null; loading: boolean }>>({});

  async function load(entry: MachineConnection) {
    cards[entry.id] = { card: cards[entry.id]?.card ?? null, loading: true };
    const card = await loadStorageCard(entry, home);
    cards[entry.id] = { card, loading: false };
  }

  $effect(() => {
    for (const entry of machines.list) if (!untrack(() => cards[entry.id])) untrack(() => void load(entry));
  });

  const loading = $derived(Object.values(cards).some((entry) => entry.loading));
  const measuredAt = (iso: string) => new Date(iso).toLocaleString([], { dateStyle: "medium", timeStyle: "short" });
  const button = "h-8 rounded-md border px-3 text-xs whitespace-nowrap transition-colors hover:bg-accent disabled:opacity-50";
</script>

<div class="flex items-start gap-4 px-1">
  <p class="min-w-0 flex-1 text-xs text-muted-foreground">
    The disks of each of your computers, and the space each project takes there. A computer that's offline shows what it measured last.
  </p>
  <button type="button" class="{button} flex shrink-0 items-center gap-1.5" disabled={loading} onclick={() => machines.list.forEach((entry) => void load(entry))}>
    <RefreshCw class="size-3.5 {loading ? 'animate-spin' : ''}" />
    Measure again
  </button>
</div>

{#each machines.list as entry (entry.id)}
  {@const shown = cards[entry.id]}
  {@const card = shown?.card ?? null}
  {@const report = card && card.state !== "none" ? card.report : null}
  {@const status = !card || shown?.loading ? "Measuring…" : card.state === "stale" ? `Last measured ${measuredAt(card.report.measuredAt)}` : null}
  <div role="group" aria-label={entry.name} class="transition-opacity {card && card.state !== 'fresh' ? 'opacity-60' : ''}" data-testid="storage-card">
    <SettingsGroup title={entry.name} description={[entry.machine.via === "this" ? "This computer" : null, status].filter(Boolean).join(" · ") || undefined}>
      {#if card && card.state !== "fresh"}
        <p class="p-4 text-xs text-muted-foreground">{card.problem}</p>
      {/if}
      {#if report}
        <ul class="flex flex-col gap-3 p-4" aria-label="Disks on {entry.name}">
          {#each report.disks as disk (disk.path)}
            {@const used = disk.totalBytes - disk.freeBytes}
            {@const low = disk.freeBytes < disk.totalBytes * 0.1}
            <li>
              <div class="flex items-baseline justify-between gap-3 text-xs">
                <span class="min-w-0 truncate font-medium" title={disk.path}>{disk.label}</span>
                <span class="shrink-0 tabular-nums {low ? 'text-destructive' : 'text-muted-foreground'}">
                  {formatBytes(disk.freeBytes)} free of {formatBytes(disk.totalBytes)}
                </span>
              </div>
              <div
                class="mt-1.5 h-2 overflow-hidden rounded-full bg-border"
                role="meter"
                aria-label="{disk.label} used"
                aria-valuemin={0}
                aria-valuemax={disk.totalBytes}
                aria-valuenow={used}
                aria-valuetext="{formatBytes(used)} used of {formatBytes(disk.totalBytes)}"
              >
                <div class="h-full rounded-full {low ? 'bg-destructive' : 'bg-primary'}" style:width="{disk.totalBytes ? (used / disk.totalBytes) * 100 : 0}%"></div>
              </div>
            </li>
          {/each}
        </ul>
        {#if report.projects.length}
          <ul class="divide-y" aria-label="Projects on {entry.name}">
            {#each report.projects as project (project.projectId)}
              <li class="flex items-center justify-between gap-3 px-4 py-2">
                <span class="min-w-0 truncate text-sm">{project.name}</span>
                <span class="shrink-0 text-xs tabular-nums text-muted-foreground">
                  {project.localBytes === null ? "Folder not found" : formatBytes(project.localBytes)}
                </span>
              </li>
            {/each}
          </ul>
        {:else}
          <p class="px-4 py-3 text-xs text-muted-foreground">No projects on {entry.name}.</p>
        {/if}
      {:else if !card}
        <p class="p-4 text-xs text-muted-foreground">Measuring…</p>
      {/if}
    </SettingsGroup>
  </div>
{/each}
