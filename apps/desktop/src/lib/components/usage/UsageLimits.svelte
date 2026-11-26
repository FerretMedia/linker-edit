<script lang="ts">
  // T3 Code's apps/web/src/components/usage/UsageLimits.tsx and UsageLimitsPooled.tsx, for one account per
  // provider (MIT, see NOTICE).
  import Gauge from "@lucide/svelte/icons/gauge";
  import TrendingDown from "@lucide/svelte/icons/trending-down";
  import TrendingUp from "@lucide/svelte/icons/trending-up";
  import TriangleAlert from "@lucide/svelte/icons/triangle-alert";
  import type { UsageLimits, UsageLimitWindow } from "#shared/usage.ts";
  import {
    formatResetsIn,
    PACE_LABEL,
    paceOf,
    paceRemainingPercent,
    PROVIDER_PRESENTATION,
    remainingPercent,
    sortLimitWindows,
    type LimitPace,
  } from "#lib/usage.ts";
  import ProviderMark from "./ProviderMark.svelte";

  let { limits, now }: { limits: UsageLimits[]; now: number } = $props();

  const PACE_ICON: Record<LimitPace, typeof Gauge> = { ahead: TrendingUp, on: Gauge, under: TrendingDown };
  const shown = $derived(limits.filter((entry) => entry.windows.length > 0));
  const notices = $derived(limits.flatMap((entry) => (entry.unavailable ? [`${PROVIDER_PRESENTATION[entry.provider].label}: ${entry.unavailable}`] : [])));

  // The tick sits in a gap cut from the bar, clamped so it stays inside at 0% and 100%.
  function paceTrack(timeLeft: number | null) {
    if (timeLeft === null) return null;
    const at = `clamp(3px, ${timeLeft}%, calc(100% - 3px))`;
    const [left, right] = [`calc(${at} - 3px)`, `calc(${at} + 3px)`];
    return {
      at,
      track: `linear-gradient(to right, var(--color-muted) ${left}, transparent ${left}, transparent ${right}, var(--color-muted) ${right})`,
      gap: `polygon(-1px -1px, ${left} -1px, ${left} calc(100% + 1px), ${right} calc(100% + 1px), ${right} -1px, calc(100% + 1px) -1px, calc(100% + 1px) calc(100% + 1px), -1px calc(100% + 1px))`,
    };
  }

  function segmentLabel(window: UsageLimitWindow, provider: string) {
    const timeLeft = paceRemainingPercent(window, now);
    return [
      `${provider}: ${remainingPercent(window)}% left`,
      formatResetsIn(window, now),
      timeLeft === null ? null : `${timeLeft}% of the window left`,
    ]
      .filter(Boolean)
      .join(", ");
  }
</script>

<div class="flex flex-col gap-8">
  {#if shown.length === 0 && notices.length === 0}
    <p class="text-sm text-muted-foreground">No provider on this computer reports subscription limits.</p>
  {/if}

  {#each shown as entry (entry.provider)}
    {@const presentation = PROVIDER_PRESENTATION[entry.provider]}
    <section class="flex flex-col gap-3">
      <h2 class="flex items-center gap-2 text-sm font-medium text-foreground">
        <span class="flex size-5 items-center justify-center"><ProviderMark provider={entry.provider} class="size-4" /></span>
        {presentation.label}
      </h2>
      {#each sortLimitWindows(entry.windows) as window (window.id)}
        {@const pace = paceOf(window, now)}
        {@const remaining = remainingPercent(window)}
        {@const mark = paceTrack(paceRemainingPercent(window, now))}
        {@const refill = formatResetsIn(window, now, "refill")}
        {@const PaceIcon = pace ? PACE_ICON[pace] : null}
        <div class="grid items-center gap-x-6 gap-y-3 rounded-lg border border-border/60 p-4 md:grid-cols-[11rem_minmax(0,1fr)]">
          <div class="flex flex-col gap-1">
            <span class="text-sm font-medium text-foreground">{window.label}</span>
            <span class="flex items-baseline gap-2">
              <span class="text-3xl font-semibold text-foreground tabular-nums">{remaining}%</span>
              <span class="inline-flex items-center gap-2 text-sm text-muted-foreground">
                <span>left</span>
                {#if pace && PaceIcon}
                  <span role="img" aria-label={PACE_LABEL[pace]} title={PACE_LABEL[pace]} class="inline-flex text-muted-foreground">
                    <PaceIcon class="size-3.5" aria-hidden="true" />
                  </span>
                {/if}
              </span>
            </span>
          </div>
          <div class="@container/pool min-w-0">
            <div
              role="img"
              aria-label={segmentLabel(window, presentation.label)}
              title={segmentLabel(window, presentation.label)}
              class="relative h-5 min-w-0 rounded-md bg-muted @2xl/pool:h-8"
              style:background-image={mark?.track}
              style:background-color={mark ? "transparent" : undefined}
            >
              <div aria-hidden="true" class="absolute inset-0 overflow-hidden rounded-md">
                <div class="absolute inset-0" style:clip-path={mark?.gap}>
                  <div class="absolute inset-y-0 left-0 rounded-md opacity-35" style:width="{remaining}%" style:background-color={presentation.color}></div>
                </div>
              </div>
              {#if mark}
                <span aria-hidden="true" class="absolute inset-y-0.5 w-0.5 -translate-x-1/2 rounded-full bg-foreground" style:left={mark.at}></span>
              {/if}
              {#if refill}
                <div aria-hidden="true" class="relative hidden h-full min-w-0 items-center gap-1.5 overflow-hidden rounded-md px-2 text-xs @2xl/pool:flex">
                  <span class="ms-auto flex min-w-0 items-center gap-1.5 overflow-hidden rounded-sm bg-background/85 px-1.5 py-0.5 text-2xs whitespace-nowrap text-foreground tabular-nums">
                    {refill}
                  </span>
                </div>
              {/if}
            </div>
          </div>
        </div>
      {/each}
    </section>
  {/each}

  {#if notices.length > 0}
    <div role="alert" class="flex items-start gap-2 rounded-lg border border-warning/40 bg-warning/8 px-3 py-2 text-sm text-warning-foreground">
      <TriangleAlert class="mt-0.5 size-4 shrink-0" />
      <div class="flex min-w-0 flex-col gap-1">
        {#each notices as notice (notice)}
          <span class="break-words">{notice}</span>
        {/each}
      </div>
    </div>
  {/if}
</div>
