<script lang="ts">
  // T3 Code's usage page, apps/web/src/components/usage/UsagePage.tsx (MIT, see NOTICE), for one computer's
  // Claude Code and Codex.
  import Check from "@lucide/svelte/icons/check";
  import ChevronDown from "@lucide/svelte/icons/chevron-down";
  import ChevronRight from "@lucide/svelte/icons/chevron-right";
  import Info from "@lucide/svelte/icons/info";
  import RefreshCw from "@lucide/svelte/icons/refresh-cw";
  import X from "@lucide/svelte/icons/x";
  import { untrack } from "svelte";
  import { USAGE_PROVIDERS, type UsageLimits as Limits, type UsageProvider, type UsageReport, type UsageWindow } from "#shared/usage.ts";
  import type { Host } from "#lib/host.ts";
  import {
    costTypeSegments,
    formatDayShort,
    formatHourShort,
    formatPercent,
    formatTokens,
    formatUsd,
    isModelCostUnknown,
    makeWindow,
    METRIC_OPTIONS,
    PROVIDER_PRESENTATION,
    readUsagePreferences,
    saveUsagePreferences,
    sessionsLabel,
    speedCostSegments,
    tokenTypeSegments,
    usageQuery,
    windowLabel,
    windowPeriods,
    WINDOW_OPTIONS,
    type UsageMetric,
    type UsagePreferences,
    type WindowDays,
  } from "#lib/usage.ts";
  import Segmented from "../Segmented.svelte";
  import ProviderMark from "./ProviderMark.svelte";
  import UsageChart from "./UsageChart.svelte";
  import UsageLimits from "./UsageLimits.svelte";
  import UsageShareBar from "./UsageShareBar.svelte";

  let { host, computer, onClose }: { host: Host; computer: string; onClose: () => void } = $props();

  let dialog: HTMLDialogElement;
  let preferences = $state<UsagePreferences>(readUsagePreferences(localStorage));
  let usageWindow = $state<UsageWindow>(makeWindow(untrack(() => preferences.windowDays)));
  const metric = $derived(preferences.metric);
  const showingLimits = $derived(metric === "limits");
  const chartMetric = $derived(metric === "tokens" ? "tokens" : "cost");
  const hidden = $derived(new Set(preferences.hiddenProviders));

  /** The last answer, kept on screen and dimmed while the next one loads. */
  let shown = $state<{ window: UsageWindow; report: UsageReport } | null>(null);
  let loading = $state(false);
  let problem = $state<string | null>(null);
  let limits = $state<Limits[] | null>(null);
  let limitsLoading = $state(false);
  let limitsProblem = $state<string | null>(null);
  // Advanced only on a refresh, so the page doesn't repaint from a ticking clock.
  let limitsNow = $state(Date.now());
  let breakdown = $state<"model" | "time">("model");
  let providerMenu = $state(false);

  $effect(() => {
    // The native dialog traps focus and closes on Escape; it has to be opened from script to be modal.
    dialog.showModal();
  });

  function update(patch: Partial<UsagePreferences>) {
    preferences = { ...preferences, ...patch };
    saveUsagePreferences(localStorage, preferences);
  }

  function selectWindow(days: WindowDays) {
    update({ windowDays: days });
    usageWindow = makeWindow(days);
  }

  function selectMetric(next: UsageMetric) {
    if (next === "limits") limitsNow = Date.now();
    update({ metric: next });
  }

  async function readJson<T>(path: string): Promise<T> {
    const res = await host.api(path, { cache: "no-store" });
    const body = (await res.json().catch(() => null)) as (T & { error?: string }) | null;
    if (!res.ok || body === null) throw new Error(body?.error ?? `The server answered ${res.status}.`);
    return body;
  }

  let usageRequest = 0;
  async function loadUsage(window: UsageWindow, hiddenProviders: UsageProvider[]) {
    const id = ++usageRequest;
    loading = true;
    const providers = USAGE_PROVIDERS.filter((provider) => !hiddenProviders.includes(provider));
    const query = [usageQuery(window), ...providers.map((provider) => `provider=${provider}`)].join("&");
    try {
      const report = await readJson<UsageReport>(`/api/usage?${query}`);
      if (id === usageRequest) {
        shown = { window, report };
        problem = null;
      }
    } catch (error) {
      if (id === usageRequest) problem = `Could not read usage: ${error instanceof Error ? error.message : String(error)}`;
    } finally {
      if (id === usageRequest) loading = false;
    }
  }

  async function loadLimits() {
    if (limitsLoading) return;
    limitsLoading = true;
    try {
      limits = await readJson<Limits[]>("/api/usage/limits");
      limitsProblem = null;
    } catch (error) {
      limitsProblem = `Could not read limits: ${error instanceof Error ? error.message : String(error)}`;
    } finally {
      limitsNow = Date.now();
      limitsLoading = false;
    }
  }

  $effect(() => {
    // Cost and tokens share one answer; a new window or provider filter asks again.
    if (!showingLimits) {
      const window = usageWindow;
      const hiddenProviders = preferences.hiddenProviders;
      untrack(() => void loadUsage(window, hiddenProviders));
    }
  });

  $effect(() => {
    if (showingLimits) untrack(() => void loadLimits());
  });

  function refresh() {
    if (showingLimits) void loadLimits();
    else {
      usageWindow = makeWindow(preferences.windowDays);
    }
  }
  const refreshing = $derived(showingLimits ? limitsLoading : loading);

  function onKeydown(event: KeyboardEvent) {
    if (event.defaultPrevented || event.repeat || event.isComposing) return;
    const target = event.target as HTMLElement | null;
    if (target?.closest("input, textarea, select, [contenteditable]")) return;
    const mod = event.ctrlKey || event.metaKey;
    if (!mod && !event.altKey && !event.shiftKey) {
      const option = METRIC_OPTIONS.find((candidate) => candidate.key === event.key.toLowerCase());
      if (option) {
        event.preventDefault();
        selectMetric(option.value);
      }
    } else if (mod && event.shiftKey && !showingLimits) {
      // event.code, since Shift turns the digits into symbols.
      const option = WINDOW_OPTIONS.find((candidate) => event.code === `Digit${candidate.key}`);
      if (option) {
        event.preventDefault();
        selectWindow(option.days);
      }
    }
  }

  const report = $derived(shown?.report ?? null);
  const shownWindow = $derived(shown?.window ?? usageWindow);
  const hourly = $derived(shownWindow.resolution === "hour");
  const periods = $derived(windowPeriods(shownWindow));
  // The providers with usage, most first by the metric shown.
  const ranked = $derived(
    (report?.providers ?? [])
      .filter((entry) => entry.totalTokens > 0 || entry.costUsd > 0)
      .toSorted((a, b) => (chartMetric === "cost" ? b.costUsd - a.costUsd : b.totalTokens - a.totalTokens)),
  );
  const activeProviders = $derived(USAGE_PROVIDERS.filter((provider) => ranked.some((entry) => entry.provider === provider)));
  const breakdownModels = $derived(
    chartMetric === "tokens" ? (report?.models ?? []).toSorted((a, b) => b.totalTokens - a.totalTokens || b.costUsd - a.costUsd) : (report?.models ?? []),
  );
  const breakdownPeak = $derived(breakdownModels.reduce((peak, model) => Math.max(peak, chartMetric === "tokens" ? model.totalTokens : model.costUsd), 0));
  const breakdownPeriods = $derived((report?.periods ?? []).toReversed());
  const visibleLimits = $derived(limits?.filter((entry) => !hidden.has(entry.provider)) ?? null);

  const providerLabel = $derived.by(() => {
    const visible = USAGE_PROVIDERS.filter((provider) => !hidden.has(provider));
    if (visible.length === USAGE_PROVIDERS.length) return "All providers";
    if (visible.length === 0) return "No providers";
    return visible.length === 1 ? PROVIDER_PRESENTATION[visible[0]!].label : `${visible.length} providers`;
  });

  function toggleProvider(provider: UsageProvider | "all") {
    if (provider === "all") update({ hiddenProviders: hidden.size === 0 ? [...USAGE_PROVIDERS] : [] });
    else update({ hiddenProviders: USAGE_PROVIDERS.filter((entry) => (entry === provider ? !hidden.has(entry) : hidden.has(entry))) });
  }

  const dim = (on: boolean) => `transition-opacity ${on ? "opacity-40 delay-150" : ""}`;
  const formatPeriod = (period: string) => (hourly ? formatHourShort(period, shownWindow.timeZone) : formatDayShort(period));
  const toggleClass = "flex size-8 shrink-0 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground disabled:opacity-50";
  const selectClass = "h-7 min-w-0 rounded-md bg-transparent px-1.5 text-xs text-foreground hover:bg-accent";
</script>

<svelte:window onkeydown={onKeydown} />

{#snippet refreshButton()}
  <button
    type="button"
    class={toggleClass}
    aria-label={showingLimits ? "Refresh limits" : "Refresh usage"}
    aria-busy={refreshing}
    disabled={refreshing}
    onclick={refresh}
  >
    <RefreshCw class="size-4 {refreshing ? 'animate-spin' : ''}" />
  </button>
{/snippet}

{#snippet metricFigure(label: string, value: string)}
  <div class="flex min-w-0 flex-col gap-0.5">
    <dt class="text-xs text-muted-foreground">{label}</dt>
    <dd class="text-base font-medium text-foreground tabular-nums {dim(loading)}">{value}</dd>
  </div>
{/snippet}

{#snippet skeleton(className: string)}
  <div class="animate-pulse rounded-md bg-accent {className}"></div>
{/snippet}

<!-- Not Dialog.svelte: T3 Code's usage page fills the window, with a header of its own. -->
<dialog
  bind:this={dialog}
  aria-label="Usage"
  class="m-0 h-dvh max-h-none w-dvw max-w-none bg-transparent p-0 text-foreground backdrop:bg-black/40"
  onclose={onClose}
  oncancel={(event) => {
    if (providerMenu) {
      event.preventDefault();
      providerMenu = false;
    }
  }}
>
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div
    class="flex size-full items-center justify-center sm:p-6 sm:pt-[max(1.5rem,env(titlebar-area-height,0px))] lg:p-10 lg:pt-[max(2.5rem,env(titlebar-area-height,0px))]"
    onclick={(event) => event.target === event.currentTarget && dialog.close()}
  >
    <!-- Takes the focus the dialog gives on opening, so no control looks picked, as in T3 Code. -->
    <!-- svelte-ignore a11y_autofocus -->
    <div
      tabindex="-1"
      autofocus
      class="relative flex size-full min-h-0 min-w-0 flex-col overflow-hidden bg-background outline-none sm:max-h-[60rem] sm:max-w-[80rem] sm:rounded-2xl sm:border sm:shadow-2xl sm:shadow-black/40"
    >
      <header class="@container/usage-header flex min-h-12 shrink-0 items-center gap-1 border-b ps-5 pe-2">
        <div class="grid w-full min-w-0 grid-cols-[minmax(0,1fr)_auto] items-center gap-x-3 py-2 @4xl/usage-header:flex">
          <nav aria-label="Usage breadcrumb" class="min-w-0">
            <ol class="flex min-w-0 items-center gap-1.5 text-sm">
              <li class="shrink-0"><h1 class="font-medium">Usage</h1></li>
              <!-- On a phone there's only room for the provider filter. -->
              <li aria-hidden="true" class="hidden shrink-0 text-muted-foreground/60 @2xl/usage-header:block"><ChevronRight class="size-3.5" /></li>
              <li class="hidden min-w-10 shrink truncate text-muted-foreground @2xl/usage-header:block" title="Usage of Claude Code and Codex on {computer}">
                {computer}
              </li>
              <li aria-hidden="true" class="shrink-0 text-muted-foreground/60"><ChevronRight class="size-3.5" /></li>
              <li class="relative min-w-0" aria-current="page">
                <button
                  type="button"
                  class="group flex max-w-full min-w-0 items-center gap-1 rounded-sm text-foreground"
                  aria-haspopup="menu"
                  aria-expanded={providerMenu}
                  onclick={() => (providerMenu = !providerMenu)}
                >
                  <span class="min-w-0 truncate">{providerLabel}</span>
                  <ChevronDown class="size-3.5 shrink-0 text-muted-foreground opacity-60 transition-opacity group-hover:opacity-100" />
                </button>
                {#if providerMenu}
                  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
                  <div class="fixed inset-0 z-10" onclick={() => (providerMenu = false)}></div>
                  <div role="menu" class="absolute top-full left-0 z-20 mt-1 w-48 rounded-lg border bg-popover p-1 text-sm shadow-lg shadow-black/10">
                    {#each [{ value: "all" as const, label: "All providers", checked: hidden.size === 0 }, ...USAGE_PROVIDERS.map( (provider) => ({ value: provider, label: PROVIDER_PRESENTATION[provider].label, checked: !hidden.has(provider) }), )] as item, index (item.value)}
                      {#if index === 1}<div role="separator" class="mx-1 my-1 h-px bg-border"></div>{/if}
                      <button
                        type="button"
                        role="menuitemcheckbox"
                        aria-checked={item.checked}
                        class="flex h-8 w-full items-center gap-2 rounded-md px-2 text-left hover:bg-accent"
                        onclick={() => toggleProvider(item.value)}
                      >
                        <Check class="size-3.5 shrink-0 {item.checked ? '' : 'invisible'}" />
                        {#if item.value !== "all"}<ProviderMark provider={item.value} class="size-3.5" />{/if}
                        <span class="truncate">{item.label}</span>
                      </button>
                    {/each}
                  </div>
                {/if}
              </li>
            </ol>
          </nav>
          {#if !showingLimits}
            <span class="hidden min-w-0 truncate text-xs text-muted-foreground @7xl/usage-header:block">{windowLabel(shownWindow)}</span>
          {/if}
          <div class="ms-auto hidden shrink-0 items-center justify-end gap-2 @4xl/usage-header:flex">
            <!-- Limits are always current, so they have no period. The period sits before the metric so dropping it never moves it. -->
            {#if !showingLimits}
              <Segmented
                label="Usage period"
                options={WINDOW_OPTIONS.map((option) => ({ value: String(option.days), label: option.label, title: `${option.label} (Ctrl+Shift+${option.key})` }))}
                value={String(preferences.windowDays)}
                onChange={(value) => selectWindow(Number(value) as WindowDays)}
              />
            {/if}
            <Segmented
              label="Usage metric"
              options={METRIC_OPTIONS.map((option) => ({ value: option.value, label: option.label, title: `${option.label} (${option.key.toUpperCase()})` }))}
              value={metric}
              onChange={selectMetric}
            />
            {@render refreshButton()}
          </div>
          <div class="ms-auto flex shrink-0 items-center justify-end gap-1 @4xl/usage-header:hidden">
            {#if !showingLimits}
              <select
                aria-label="Usage period"
                class={selectClass}
                value={String(preferences.windowDays)}
                onchange={(event) => selectWindow(Number(event.currentTarget.value) as WindowDays)}
              >
                {#each WINDOW_OPTIONS as option (option.days)}
                  <option value={String(option.days)}>{option.label}</option>
                {/each}
              </select>
            {/if}
            <select aria-label="Usage metric" class={selectClass} value={metric} onchange={(event) => selectMetric(event.currentTarget.value as UsageMetric)}>
              {#each METRIC_OPTIONS as option (option.value)}
                <option value={option.value}>{option.label}</option>
              {/each}
            </select>
            {@render refreshButton()}
          </div>
        </div>
        <button type="button" class={toggleClass} aria-label="Close" title="Close (Esc)" onclick={() => dialog.close()}>
          <X class="size-4" />
        </button>
      </header>

      <div class="min-h-0 flex-1 overflow-y-auto">
        <div class="mx-auto flex w-full max-w-5xl flex-col gap-6 px-5 pt-6 pb-12 sm:px-6">
          {#if showingLimits}
            {#if visibleLimits}
              <UsageLimits limits={visibleLimits} now={limitsNow} />
            {:else if limitsProblem}
              <p class="text-sm text-destructive">{limitsProblem}</p>
            {:else}
              <div class="flex flex-col gap-3">
                {@render skeleton("h-5 w-32")}
                {@render skeleton("h-24")}
                {@render skeleton("h-24")}
              </div>
            {/if}
          {:else if !report}
            {#if problem}
              <p class="py-12 text-center text-sm text-muted-foreground">{problem}</p>
            {:else}
              <section class="grid gap-6 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]">
                <div class="flex flex-col gap-5">
                  <div class="flex flex-col gap-1">{@render skeleton("h-10 w-36")}{@render skeleton("h-4 w-32")}</div>
                  {#each USAGE_PROVIDERS as provider (provider)}
                    <div class="flex flex-col gap-1">{@render skeleton("h-5 w-full")}{@render skeleton("h-4 w-36")}</div>
                  {/each}
                </div>
                <div class="flex flex-col gap-3">{@render skeleton("h-5 w-24")}{@render skeleton("ml-16 h-56")}</div>
              </section>
              {@render skeleton("h-44")}
            {/if}
          {:else}
            <div aria-busy={loading} class="flex flex-col gap-6">
              {#each report.notices as notice (notice)}
                <p class="text-sm text-muted-foreground">{notice}</p>
              {/each}
              {#if problem}
                <p class="text-sm text-destructive">{problem}</p>
              {/if}
              <section class="grid gap-6 lg:grid-cols-[minmax(0,18rem)_minmax(0,1fr)]">
                <div class="flex min-w-0 flex-col gap-5">
                  <div class="flex flex-col gap-1">
                    <p class="text-4xl font-semibold text-foreground tabular-nums {dim(loading)}">
                      {chartMetric === "cost" ? formatUsd(report.costUsd) : formatTokens(report.totalTokens)}
                    </p>
                    <p class="inline-flex items-center gap-1 text-xs text-muted-foreground">
                      <span><span class={dim(loading)}>{sessionsLabel(report.sessions)}</span>{chartMetric === "cost" ? " · API estimate" : ""}</span>
                      {#if chartMetric === "cost" && report.unpricedShare > 0}
                        <span
                          role="img"
                          class="inline-flex"
                          aria-label="API estimate excludes {formatPercent(report.unpricedShare)} unpriced records."
                          title="API estimate excludes {formatPercent(report.unpricedShare)} unpriced records."
                        >
                          <Info class="size-3" aria-hidden="true" />
                        </span>
                      {/if}
                    </p>
                  </div>

                  {#each ranked as totals (totals.provider)}
                    {@const presentation = PROVIDER_PRESENTATION[totals.provider]}
                    <div class="flex flex-col gap-1">
                      <dl class="flex items-baseline justify-between gap-4">
                        <dt class="flex min-w-0 items-center gap-2 text-sm text-foreground">
                          <span aria-hidden="true" class="size-2 shrink-0 rounded-full" style:background-color={presentation.color}></span>
                          <ProviderMark provider={totals.provider} class="size-4" />
                          <span class="flex min-w-0 items-baseline gap-1.5">
                            <span class="truncate">{presentation.label}</span>
                            <span class="shrink-0 text-2xs whitespace-nowrap text-muted-foreground tabular-nums {dim(loading)}">{sessionsLabel(totals.sessions)}</span>
                          </span>
                        </dt>
                        <dd class="shrink-0 text-sm font-medium text-foreground tabular-nums {dim(loading)}">
                          {chartMetric === "cost" ? formatUsd(totals.costUsd) : formatTokens(totals.totalTokens)}
                        </dd>
                      </dl>
                      <p class="text-xs text-muted-foreground {dim(loading)}">
                        {chartMetric === "cost"
                          ? `${formatPercent(totals.costShare)} of cost · ${formatTokens(totals.totalTokens)} tokens`
                          : `${formatPercent(totals.tokenShare)} of tokens · ${formatUsd(totals.costUsd)}`}
                      </p>
                    </div>
                  {/each}
                </div>

                <div class="flex min-w-0 flex-col gap-3">
                  <h2 class="text-sm font-medium text-foreground">
                    {hourly ? (chartMetric === "tokens" ? "Hourly processed tokens" : "Hourly cost") : chartMetric === "tokens" ? "Daily processed tokens" : "Daily cost"}
                  </h2>
                  <UsageChart providers={activeProviders} {periods} data={report.periods} metric={chartMetric} window={shownWindow} {loading} />
                </div>
              </section>

              <section class="flex flex-col gap-2">
                <h2 class="text-sm font-medium text-foreground">Totals</h2>
                <dl class="grid grid-cols-2 gap-x-6 gap-y-4 py-1 md:grid-cols-5">
                  {@render metricFigure("Processed tokens", formatTokens(report.totalTokens))}
                  {@render metricFigure("Cached input", formatTokens(report.cachedInputTokens))}
                  {@render metricFigure("Uncached input", formatTokens(report.uncachedInputTokens))}
                  {@render metricFigure("Output", formatTokens(report.outputTokens))}
                  {@render metricFigure("Cache savings", formatUsd(report.cacheSavingsUsd))}
                </dl>
              </section>

              {#if report.totalTokens > 0}
                <section class="grid gap-x-12 gap-y-8 lg:grid-cols-2 {dim(loading)}">
                  {#if chartMetric === "tokens"}
                    <UsageShareBar label="Tokens by type" segments={tokenTypeSegments(report)} format={formatTokens} />
                  {:else}
                    <UsageShareBar label="Cost by type" segments={costTypeSegments(report.categoryCost)} format={formatUsd} />
                    {#if report.speedCost.fast + report.speedCost.ultrafast > 0}
                      <UsageShareBar label="Cost by speed" segments={speedCostSegments(report.speedCost)} format={formatUsd}>
                        {#snippet aside()}
                          <span class="text-xs text-muted-foreground tabular-nums">Premium {formatUsd(report.speedCost.premium)}</span>
                        {/snippet}
                      </UsageShareBar>
                    {/if}
                  {/if}
                </section>
              {/if}

              <section class="flex flex-col gap-3">
                <div class="flex items-center justify-between gap-3">
                  <h2 class="text-sm font-medium text-foreground">Breakdown</h2>
                  <Segmented
                    label="Usage breakdown"
                    options={[
                      { value: "model", label: "Model" },
                      { value: "time", label: hourly ? "Hour" : "Day" },
                    ]}
                    value={breakdown}
                    onChange={(value) => (breakdown = value)}
                  />
                </div>

                {#if breakdown === "model"}
                  <table class="w-full text-sm">
                    <thead>
                      <tr class="border-b border-border text-right text-xs text-muted-foreground">
                        <th class="py-2 pr-3 pl-2 text-left font-normal">#</th>
                        <th class="w-full py-2 text-left font-normal">Model</th>
                        <th class="py-2 pl-6 font-normal">Cost</th>
                        <th class="hidden py-2 pl-6 font-normal sm:table-cell">Share</th>
                        <th class="py-2 pr-2 pl-6 font-normal">Tokens</th>
                      </tr>
                    </thead>
                    <tbody>
                      {#each breakdownModels as model, index (`${model.provider}:${model.model}`)}
                        {@const value = chartMetric === "tokens" ? model.totalTokens : model.costUsd}
                        {@const hasBar = value > 0 && breakdownPeak > 0}
                        {@const unknownCost = isModelCostUnknown(model)}
                        {@const share = chartMetric === "tokens" ? model.tokenShare : unknownCost ? null : model.costShare}
                        <tr class="border-b border-border/50 text-right whitespace-nowrap text-muted-foreground tabular-nums transition-colors hover:bg-muted/50">
                          <td class="py-2.5 pr-3 pl-2 text-left text-xs">{index + 1}</td>
                          <td class="py-2.5 text-left whitespace-normal">
                            <span class="flex items-center gap-2 text-left text-foreground {hasBar ? '' : 'py-1'}">
                              <ProviderMark provider={model.provider} class="size-3.5" />
                              {model.model}
                            </span>
                            {#if hasBar}
                              <div aria-hidden="true" class="mt-1.5 h-0.5 max-w-48 {dim(loading)}">
                                <div
                                  class="h-full rounded-full"
                                  style:width="max(0.5rem, {(value / breakdownPeak) * 100}%)"
                                  style:background-color={PROVIDER_PRESENTATION[model.provider].color}
                                ></div>
                              </div>
                            {/if}
                          </td>
                          <td class="py-2.5 pl-6 text-foreground {dim(loading)}">
                            {#if unknownCost}<span class="text-muted-foreground">Unpriced</span>{:else}{formatUsd(model.costUsd)}{/if}
                          </td>
                          <td class="hidden py-2.5 pl-6 sm:table-cell {dim(loading)}">{share === null ? "" : formatPercent(share)}</td>
                          <td class="py-2.5 pr-2 pl-6 {dim(loading)}">{formatTokens(model.totalTokens)}</td>
                        </tr>
                      {:else}
                        <tr><td colspan="5" class="py-6 text-center text-muted-foreground">No activity in this window.</td></tr>
                      {/each}
                    </tbody>
                  </table>
                {:else}
                  <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
                  <div
                    class="overflow-x-auto outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
                    role="region"
                    aria-label="Usage breakdown by time"
                    tabindex="0"
                  >
                    <table class="w-full text-sm whitespace-nowrap">
                      <colgroup><col class="w-2/5" /></colgroup>
                      <thead>
                        <tr class="border-b border-border text-left text-xs text-muted-foreground">
                          <th class="py-2 pl-2 font-normal">{hourly ? "Hour" : "Day"}</th>
                          {#each activeProviders as provider (provider)}
                            <th class="py-2 pl-4 text-right font-normal">{PROVIDER_PRESENTATION[provider].label}</th>
                          {/each}
                          <th class="py-2 pl-4 text-right font-normal">Total</th>
                          <th class="py-2 pr-2 pl-4 text-right font-normal">Tokens</th>
                        </tr>
                      </thead>
                      <tbody>
                        {#each breakdownPeriods as period (period.period)}
                          <tr class="border-b border-border/50 transition-colors hover:bg-muted/50">
                            <td class="py-2 pl-2 text-foreground">{formatPeriod(period.period)}</td>
                            {#each activeProviders as provider (provider)}
                              <td class="py-2 pl-4 text-right text-muted-foreground tabular-nums {dim(loading)}">{formatUsd(period.byProvider[provider]?.costUsd ?? 0)}</td>
                            {/each}
                            <td class="py-2 pl-4 text-right text-foreground tabular-nums {dim(loading)}">{formatUsd(period.costUsd)}</td>
                            <td class="py-2 pr-2 pl-4 text-right text-muted-foreground tabular-nums {dim(loading)}">{formatTokens(period.totalTokens)}</td>
                          </tr>
                        {:else}
                          <tr><td colspan={activeProviders.length + 3} class="py-6 text-center text-muted-foreground">No activity in this window.</td></tr>
                        {/each}
                      </tbody>
                    </table>
                  </div>
                {/if}
              </section>
            </div>
          {/if}
        </div>
      </div>
    </div>
  </div>
</dialog>
