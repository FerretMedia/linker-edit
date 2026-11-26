<script lang="ts">
  // T3 Code's apps/web/src/components/usage/UsageProviderChart.tsx (MIT, see NOTICE).
  import { USAGE_PROVIDERS, type UsagePeriod, type UsageProvider, type UsageWindow } from "#shared/usage.ts";
  import { formatDayShort, formatHourShort, formatRelativeHourShort, formatTokens, formatUsd, PROVIDER_PRESENTATION } from "#lib/usage.ts";
  import ProviderMark from "./ProviderMark.svelte";

  let {
    providers,
    periods,
    data,
    metric,
    window,
    loading,
  }: {
    /** The providers with usage, in the page's order. */
    providers: UsageProvider[];
    /** Every day or hour of the window. */
    periods: string[];
    data: UsagePeriod[];
    metric: "cost" | "tokens";
    window: UsageWindow;
    loading: boolean;
  } = $props();

  const VIEW_WIDTH = 960;
  const VIEW_HEIGHT = 260;
  const TICK_COUNT = 4;
  const PLOT_TOP = 8;

  type Point = { x: number; y: number };

  function monotoneTangents(points: Point[]): number[] {
    const count = points.length;
    if (count < 2) return [0];
    const slopes: number[] = [];
    for (let index = 0; index < count - 1; index += 1) {
      const dx = points[index + 1]!.x - points[index]!.x;
      const dy = points[index + 1]!.y - points[index]!.y;
      slopes.push(dx === 0 ? 0 : dy / dx);
    }
    const tangents = Array.from({ length: count }, () => 0);
    tangents[0] = slopes[0] ?? 0;
    tangents[count - 1] = slopes[count - 2] ?? 0;
    for (let index = 1; index < count - 1; index += 1) {
      const previous = slopes[index - 1]!;
      const next = slopes[index]!;
      tangents[index] = previous * next <= 0 ? 0 : (previous + next) / 2;
    }
    for (let index = 0; index < count - 1; index += 1) {
      const slope = slopes[index]!;
      if (slope === 0) {
        tangents[index] = 0;
        tangents[index + 1] = 0;
        continue;
      }
      const a = tangents[index]! / slope;
      const b = tangents[index + 1]! / slope;
      const magnitude = a * a + b * b;
      if (magnitude > 9) {
        const scale = 3 / Math.sqrt(magnitude);
        tangents[index] = scale * a * slope;
        tangents[index + 1] = scale * b * slope;
      }
    }
    return tangents;
  }

  /** A monotone cubic through the points, so the curve never overshoots below zero. */
  function curvePath(points: Point[]): string {
    if (points.length < 2) return "";
    const tangents = monotoneTangents(points);
    let path = `M${points[0]!.x.toFixed(2)},${points[0]!.y.toFixed(2)}`;
    for (let index = 0; index < points.length - 1; index += 1) {
      const from = points[index]!;
      const to = points[index + 1]!;
      const dx = to.x - from.x;
      const c1 = { x: from.x + dx / 3, y: from.y + (tangents[index]! * dx) / 3 };
      const c2 = { x: to.x - dx / 3, y: to.y - (tangents[index + 1]! * dx) / 3 };
      path += ` C${c1.x.toFixed(2)},${c1.y.toFixed(2)} ${c2.x.toFixed(2)},${c2.y.toFixed(2)} ${to.x.toFixed(2)},${to.y.toFixed(2)}`;
    }
    return path;
  }

  function niceScale(peak: number): { max: number; ticks: number[] } {
    if (peak <= 0) return { max: 0, ticks: [0] };
    const rawStep = peak / TICK_COUNT;
    const magnitude = 10 ** Math.floor(Math.log10(rawStep));
    const normalized = rawStep / magnitude;
    const step = (normalized > 5 ? 10 : normalized > 2 ? 5 : normalized > 1 ? 2 : 1) * magnitude;
    const max = Math.ceil(peak / step) * step;
    const ticks: number[] = [];
    for (let value = 0; value <= max + step * 1e-6; value += step) ticks.push(value);
    return { max, ticks };
  }

  const toY = (value: number, max: number) => (max === 0 ? VIEW_HEIGHT : VIEW_HEIGHT - (value / max) * (VIEW_HEIGHT - PLOT_TOP));

  const format = $derived(metric === "tokens" ? formatTokens : formatUsd);
  const byPeriod = $derived(new Map(data.map((entry) => [entry.period, entry])));
  const columns = $derived(
    periods.map((period) => {
      const entry = byPeriod.get(period);
      const bands = USAGE_PROVIDERS.map((provider) => {
        const amount = entry?.byProvider[provider];
        return { provider, value: amount ? (metric === "tokens" ? amount.totalTokens : amount.costUsd) : 0 };
      });
      return { bands, total: bands.reduce((sum, band) => sum + band.value, 0) };
    }),
  );
  // Layered series each start at zero; summing peaks would waste plot height.
  const scale = $derived(niceScale(columns.reduce((max, column) => column.bands.reduce((inner, band) => Math.max(inner, band.value), max), 0)));
  const stepX = $derived(periods.length < 2 ? 0 : VIEW_WIDTH / (periods.length - 1));
  const paths = $derived(
    providers
      .map((provider) => {
        const slot = USAGE_PROVIDERS.indexOf(provider);
        const line = curvePath(columns.map((column, index) => ({ x: index * stepX, y: toY(column.bands[slot]!.value, scale.max) })));
        return {
          provider,
          total: columns.reduce((sum, column) => sum + column.bands[slot]!.value, 0),
          line,
          area: line === "" ? "" : `${line} L${VIEW_WIDTH},${VIEW_HEIGHT} L0,${VIEW_HEIGHT} Z`,
        };
      })
      .sort((a, b) => b.total - a.total),
  );

  const formatPeriod = (period: string) => (window.resolution === "hour" ? formatHourShort(period, window.timeZone) : formatDayShort(period));
  const formatTooltipPeriod = (period: string) =>
    window.resolution === "hour" ? formatRelativeHourShort(period, window.untilTime, window.timeZone) : formatDayShort(period);
  const bandValue = (index: number, provider: UsageProvider) => columns[index]?.bands.find((band) => band.provider === provider)?.value ?? 0;

  let plot: HTMLDivElement | undefined = $state();
  let hoverIndex = $state<number | null>(null);
  let pointer = $state({ x: 0, y: 0 });
  let tooltipSize = $state({ width: 0, height: 0 });
  let plotSize = $state({ width: 0, height: 0 });

  // Beside the pointer, flipped to the other side where it would leave the plot.
  const tooltipPosition = $derived.by(() => {
    const gap = 12;
    const left = pointer.x + gap + tooltipSize.width <= plotSize.width ? pointer.x + gap : pointer.x - gap - tooltipSize.width;
    const top = pointer.y + gap + tooltipSize.height <= plotSize.height ? pointer.y + gap : pointer.y - gap - tooltipSize.height;
    return {
      left: Math.min(Math.max(0, left), Math.max(0, plotSize.width - tooltipSize.width)),
      top: Math.min(Math.max(0, top), Math.max(0, plotSize.height - tooltipSize.height)),
    };
  });

  function onMove(event: MouseEvent) {
    if (!plot || periods.length === 0) return;
    const bounds = plot.getBoundingClientRect();
    if (bounds.width === 0) return;
    pointer = {
      x: Math.min(bounds.width, Math.max(0, event.clientX - bounds.left)),
      y: Math.min(bounds.height, Math.max(0, event.clientY - bounds.top)),
    };
    plotSize = { width: plot.clientWidth, height: plot.clientHeight };
    hoverIndex = Math.min(periods.length - 1, Math.max(0, Math.round((pointer.x / bounds.width) * (periods.length - 1))));
  }

  const middle = $derived(periods[Math.floor(periods.length / 2)]);
</script>

<div class="flex flex-col gap-1">
  <div class="flex gap-2">
    <div aria-hidden="true" class="relative h-56 w-14 shrink-0">
      {#each scale.ticks as tick (tick)}
        <span
          class="absolute right-0 -translate-y-1/2 text-3xs text-muted-foreground tabular-nums"
          style:top="{(toY(tick, scale.max) / VIEW_HEIGHT) * 100}%"
        >
          {tick === 0 ? "0" : format(tick)}
        </span>
      {/each}
    </div>

    <div
      bind:this={plot}
      role="presentation"
      class="relative h-56 flex-1"
      onmousemove={onMove}
      onmouseleave={() => (hoverIndex = null)}
    >
      <svg class="h-full w-full" viewBox="0 0 {VIEW_WIDTH} {VIEW_HEIGHT}" preserveAspectRatio="none" aria-hidden="true">
        {#each scale.ticks as tick (tick)}
          <line
            x1="0"
            x2={VIEW_WIDTH}
            y1={toY(tick, scale.max)}
            y2={toY(tick, scale.max)}
            stroke="currentColor"
            stroke-width="1"
            class="text-border"
            vector-effect="non-scaling-stroke"
          />
        {/each}
        {#each paths as path (path.provider)}
          <path
            d={path.area}
            class="transition-opacity {loading ? 'opacity-40 delay-150' : ''}"
            fill={PROVIDER_PRESENTATION[path.provider].color}
            fill-opacity="0.12"
          />
        {/each}
        {#each paths as path (path.provider)}
          <path
            d={path.line}
            class="transition-opacity {loading ? 'opacity-40 delay-150' : ''}"
            fill="none"
            stroke={PROVIDER_PRESENTATION[path.provider].color}
            stroke-width="2"
            vector-effect="non-scaling-stroke"
          />
        {/each}
        {#if hoverIndex !== null}
          <line
            x1={hoverIndex * stepX}
            x2={hoverIndex * stepX}
            y1={PLOT_TOP}
            y2={VIEW_HEIGHT}
            stroke="currentColor"
            stroke-width="1"
            class="text-muted-foreground"
            vector-effect="non-scaling-stroke"
          />
        {/if}
      </svg>

      {#if hoverIndex !== null && periods[hoverIndex] !== undefined}
        <div
          bind:offsetWidth={tooltipSize.width}
          bind:offsetHeight={tooltipSize.height}
          class="pointer-events-none absolute z-10 max-w-full min-w-36 rounded-xl border border-border/50 bg-popover/95 px-2.5 py-2 text-xs shadow-lg backdrop-blur"
          style:left="{tooltipPosition.left}px"
          style:top="{tooltipPosition.top}px"
        >
          <div class="mb-1 text-muted-foreground">{formatTooltipPeriod(periods[hoverIndex]!)}</div>
          {#each providers as provider (provider)}
            <div class="flex items-center justify-between gap-3">
              <span class="flex items-center gap-1.5 text-muted-foreground">
                <ProviderMark {provider} class="size-3" />
                {PROVIDER_PRESENTATION[provider].label}
              </span>
              <span class="tabular-nums {loading ? 'text-muted-foreground' : 'text-foreground'}">{format(bandValue(hoverIndex, provider))}</span>
            </div>
          {/each}
          <div class="mt-1 flex items-center justify-between gap-3 border-t border-border pt-1">
            <span class="text-muted-foreground">Total</span>
            <span class="tabular-nums {loading ? 'text-muted-foreground' : 'text-foreground'}">{format(columns[hoverIndex]?.total ?? 0)}</span>
          </div>
        </div>
      {/if}
    </div>
  </div>

  <div aria-hidden="true" class="flex justify-between pl-16 text-3xs text-muted-foreground uppercase">
    <span>{periods[0] === undefined ? "" : formatPeriod(periods[0])}</span>
    <span>{middle === undefined ? "" : formatPeriod(middle)}</span>
    <span>{periods.at(-1) === undefined ? "" : formatPeriod(periods.at(-1)!)}</span>
  </div>

  <!-- The plot and its axes are hover-only; this carries the same figures for screen readers. -->
  <div class="sr-only">
    <table>
      <caption>
        {window.resolution === "hour" ? (metric === "tokens" ? "Hourly processed tokens by provider" : "Hourly cost by provider") : metric === "tokens" ? "Daily processed tokens by provider" : "Daily cost by provider"}
      </caption>
      <thead>
        <tr>
          <th scope="col">{window.resolution === "hour" ? "Hour" : "Day"}</th>
          {#each providers as provider (provider)}
            <th scope="col">{PROVIDER_PRESENTATION[provider].label}</th>
          {/each}
          {#if providers.length > 1}<th scope="col">Total</th>{/if}
        </tr>
      </thead>
      <tbody>
        {#each periods as period, index (period)}
          <tr>
            <th scope="row">{formatTooltipPeriod(period)}</th>
            {#each providers as provider (provider)}
              <td>{format(bandValue(index, provider))}</td>
            {/each}
            {#if providers.length > 1}<td>{format(columns[index]?.total ?? 0)}</td>{/if}
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>
