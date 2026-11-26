// The usage page's numbers and words, from T3 Code's packages/shared/src/usageFormat.ts, usageLimits.ts,
// usageLimitsCopy.ts and apps/web/src/components/usage/usageBreakdown.ts, usageProviders.ts,
// usagePagePreferences.ts and usageShortcuts.ts (MIT, see NOTICE).
import { USAGE_PROVIDERS, type UsageLimitWindow, type UsageProvider, type UsageReport, type UsageWindow } from "#shared/usage.ts";
import { PROVIDER_LABEL } from "./labels.ts";

export const PROVIDER_PRESENTATION: Record<UsageProvider, { label: string; color: string }> = {
  codex: { label: PROVIDER_LABEL.codex, color: "var(--foreground)" },
  claude: { label: PROVIDER_LABEL.claude, color: "#d97757" },
};

export type UsageMetric = "cost" | "tokens" | "limits";

export const METRIC_OPTIONS = [
  { value: "cost", label: "Cost", key: "c" },
  { value: "tokens", label: "Tokens", key: "t" },
  { value: "limits", label: "Limits", key: "l" },
] as const;

export const WINDOW_OPTIONS = [
  { days: 1, label: "Past 24h", key: "1" },
  { days: 7, label: "7 days", key: "2" },
  { days: 30, label: "30 days", key: "3" },
  { days: 90, label: "90 days", key: "4" },
] as const;
export type WindowDays = (typeof WINDOW_OPTIONS)[number]["days"];

export interface UsagePreferences {
  metric: UsageMetric;
  windowDays: WindowDays;
  hiddenProviders: UsageProvider[];
}

const PREFERENCES_KEY = "linker-edit:usage-page";
const DEFAULT_PREFERENCES: UsagePreferences = { metric: "limits", windowDays: 30, hiddenProviders: [] };

export function readUsagePreferences(storage: Pick<Storage, "getItem">): UsagePreferences {
  try {
    const saved = JSON.parse(storage.getItem(PREFERENCES_KEY) ?? "null") as Partial<UsagePreferences> | null;
    return {
      metric: METRIC_OPTIONS.some((option) => option.value === saved?.metric) ? saved!.metric! : DEFAULT_PREFERENCES.metric,
      windowDays: WINDOW_OPTIONS.some((option) => option.days === saved?.windowDays) ? saved!.windowDays! : DEFAULT_PREFERENCES.windowDays,
      hiddenProviders: Array.isArray(saved?.hiddenProviders) ? USAGE_PROVIDERS.filter((provider) => saved.hiddenProviders!.includes(provider)) : [],
    };
  } catch {
    return DEFAULT_PREFERENCES;
  }
}

export function saveUsagePreferences(storage: Pick<Storage, "setItem">, preferences: UsagePreferences) {
  try {
    storage.setItem(PREFERENCES_KEY, JSON.stringify(preferences));
  } catch {
    // Private windows and full storage only cost the choice after a reload.
  }
}

// ---- Windows ----

const HOUR_MS = 60 * 60 * 1000;

function dayFormat(timeZone: string) {
  return new Intl.DateTimeFormat("en-CA", { timeZone, year: "numeric", month: "2-digit", day: "2-digit" });
}

/** The last `days` calendar days in the viewer's zone, or for one day the 24 hours up to now. */
export function makeWindow(days: number, now = new Date()): UsageWindow {
  let timeZone = Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  let format: Intl.DateTimeFormat;
  try {
    format = dayFormat(timeZone);
  } catch {
    timeZone = "UTC";
    format = dayFormat(timeZone);
  }
  if (days === 1) {
    const untilMs = Math.floor(now.getTime() / 60_000) * 60_000;
    return { resolution: "hour", sinceTime: new Date(untilMs - 24 * HOUR_MS).toISOString(), untilTime: new Date(untilMs).toISOString(), timeZone };
  }
  const untilDay = format.format(now);
  // Calendar arithmetic in UTC; subtracting milliseconds crosses DST days wrongly.
  const [year = 0, month = 1, dayOfMonth = 1] = untilDay.split("-").map(Number);
  const sinceDay = new Date(Date.UTC(year, month - 1, dayOfMonth - (days - 1))).toISOString().slice(0, 10);
  return { resolution: "day", sinceDay, untilDay, timeZone };
}

export function usageQuery(window: UsageWindow): string {
  const params: Record<string, string> =
    window.resolution === "hour"
      ? { from: window.sinceTime, to: window.untilTime, tz: window.timeZone }
      : { since: window.sinceDay, until: window.untilDay, tz: window.timeZone };
  return new URLSearchParams(params).toString();
}

/** Every period of the window, so days without usage still show on the chart. */
export function windowPeriods(window: UsageWindow): string[] {
  const periods: string[] = [];
  if (window.resolution === "hour") {
    const end = Date.parse(window.untilTime);
    for (let at = Date.parse(window.sinceTime); at < end; at += HOUR_MS) periods.push(new Date(at).toISOString());
    return periods;
  }
  const end = Date.parse(`${window.untilDay}T00:00:00Z`);
  for (let at = Date.parse(`${window.sinceDay}T00:00:00Z`); at <= end; at += 24 * HOUR_MS) periods.push(new Date(at).toISOString().slice(0, 10));
  return periods;
}

// ---- Numbers ----

const CURRENCY = new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 2 });
const INTEGER = new Intl.NumberFormat("en-US");

export const formatUsd = (value: number) => CURRENCY.format(value);
export const formatCount = (value: number) => INTEGER.format(Math.round(value));

function trim(value: number): string {
  const abs = Math.abs(value);
  const digits = abs >= 100 ? 0 : abs >= 10 ? 1 : 2;
  return value.toFixed(digits).replace(/\.0+$/, "");
}

const TOKEN_SUFFIXES = ["K", "M", "B", "T"] as const;

export function formatTokens(value: number): string {
  if (Math.abs(Math.round(value)) < 1e3) return INTEGER.format(Math.round(value));
  let scaled = value / 1e3;
  let unit = 0;
  // The unit is chosen after rounding, so 999,999 reads 1M rather than 1000K.
  while (Math.abs(Number(trim(scaled))) >= 1e3 && unit < TOKEN_SUFFIXES.length - 1) {
    scaled /= 1e3;
    unit += 1;
  }
  return `${trim(scaled)}${TOKEN_SUFFIXES[unit]}`;
}

export function formatPercent(share: number, digits = 1): string {
  const percent = share * 100;
  const smallest = 10 ** -digits;
  if (percent > 0 && percent < smallest) return `<${smallest.toFixed(digits)}%`;
  return `${percent.toFixed(digits)}%`;
}

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

export function formatDayShort(day: string): string {
  const [, month, dayOfMonth] = day.split("-").map(Number);
  return month && dayOfMonth ? `${MONTHS[month - 1]} ${dayOfMonth}` : day;
}

/** "3 PM"; rolling hours can start between whole hours, but their labels name the hour. */
export function formatHourShort(hourStart: string, timeZone: string): string {
  return new Intl.DateTimeFormat("en-US", { timeZone, hour: "numeric", hour12: true }).format(new Date(hourStart));
}

export function formatDateTimeShort(instant: string, timeZone: string): string {
  return new Intl.DateTimeFormat("en-US", { timeZone, month: "short", day: "numeric", hour: "numeric", hour12: true }).format(new Date(instant));
}

/** "3 PM today", "3 PM yesterday", or a date further back. */
export function formatRelativeHourShort(hourStart: string, relativeTo: string, timeZone: string): string {
  const format = dayFormat(timeZone);
  const daysAgo = Math.round((Date.parse(format.format(new Date(relativeTo))) - Date.parse(format.format(new Date(hourStart)))) / (24 * HOUR_MS));
  const hour = formatHourShort(hourStart, timeZone);
  if (daysAgo === 0) return `${hour} today`;
  if (daysAgo === 1) return `${hour} yesterday`;
  return formatDateTimeShort(hourStart, timeZone);
}

export function windowLabel(window: UsageWindow): string {
  return window.resolution === "hour"
    ? `${formatDateTimeShort(window.sinceTime, window.timeZone)} to ${formatDateTimeShort(window.untilTime, window.timeZone)}`
    : `${formatDayShort(window.sinceDay)} to ${formatDayShort(window.untilDay)}`;
}

export const sessionsLabel = (count: number) => `${formatCount(count)} ${count === 1 ? "session" : "sessions"}`;

// ---- Breakdown ----

export interface ShareSegment {
  label: string;
  value: number;
  color: string;
}

const ink = (percent: number) => `color-mix(in oklab, var(--foreground) ${percent}%, var(--background))`;
const TYPE_COLORS = { input: ink(60), cacheRead: ink(30), cacheWrite: ink(72), output: ink(100), other: ink(44) };

export function costTypeSegments(cost: UsageReport["categoryCost"]): ShareSegment[] {
  return [
    { label: "Input", value: cost.input, color: TYPE_COLORS.input },
    { label: "Cache read", value: cost.cacheRead, color: TYPE_COLORS.cacheRead },
    { label: "Cache write", value: cost.cacheWrite, color: TYPE_COLORS.cacheWrite },
    { label: "Output", value: cost.output, color: TYPE_COLORS.output },
    { label: "Other", value: cost.unsplit >= 0.005 ? cost.unsplit : 0, color: TYPE_COLORS.other },
  ];
}

export function tokenTypeSegments(report: UsageReport): ShareSegment[] {
  return [
    { label: "Input", value: report.uncachedInputTokens, color: TYPE_COLORS.input },
    { label: "Cache read", value: report.cachedInputTokens, color: TYPE_COLORS.cacheRead },
    { label: "Cache write", value: report.cacheCreationTokens, color: TYPE_COLORS.cacheWrite },
    { label: "Output", value: report.outputTokens, color: TYPE_COLORS.output },
  ];
}

export function speedCostSegments(cost: UsageReport["speedCost"]): ShareSegment[] {
  return [
    { label: "Standard", value: cost.standard, color: ink(34) },
    { label: "Fast", value: cost.fast, color: ink(66) },
    { label: "Ultrafast", value: cost.ultrafast, color: ink(100) },
  ];
}

/** When every record lacks rates, the cost is unknown and isn't shown as a dollar figure. */
export const isModelCostUnknown = (model: UsageReport["models"][number]) => model.records > 0 && model.unpricedRecords >= model.records;

// ---- Limits ----

const MINUTE = 60_000;
const DAY = 24 * HOUR_MS;

export const remainingPercent = (window: UsageLimitWindow) => Math.round(100 - Math.max(0, Math.min(100, window.usedPercent)));

function elapsedShare(window: UsageLimitWindow, now: number): number | null {
  const resetsAt = window.resetsAt ? Date.parse(window.resetsAt) : NaN;
  if (Number.isNaN(resetsAt) || window.windowDurationMins === undefined) return null;
  const length = window.windowDurationMins * MINUTE;
  if (length <= 0) return null;
  return Math.max(0, Math.min(1, (length - (resetsAt - now)) / length));
}

/** Bars show quota remaining, so even pace tracks the time remaining. */
export function paceRemainingPercent(window: UsageLimitWindow, now: number): number | null {
  const elapsed = elapsedShare(window, now);
  return elapsed === null ? null : Math.round((1 - elapsed) * 100);
}

export type LimitPace = "ahead" | "on" | "under";

export const PACE_LABEL: Record<LimitPace, string> = {
  ahead: "Ahead of pace: spending faster than the window elapses",
  on: "On pace with the window",
  under: "Under pace: headroom left for the rest of the window",
};

export function paceOf(window: UsageLimitWindow, now: number): LimitPace | null {
  const elapsed = elapsedShare(window, now);
  if (elapsed === null) return null;
  const gap = window.usedPercent - elapsed * 100;
  return gap > 5 ? "ahead" : gap < -5 ? "under" : "on";
}

export function formatDuration(ms: number): string {
  const remaining = Math.max(0, ms);
  const days = Math.floor(remaining / DAY);
  const hours = Math.floor((remaining % DAY) / HOUR_MS);
  const minutes = Math.floor((remaining % HOUR_MS) / MINUTE);
  if (days > 0) return `${days}d ${hours}h`;
  if (hours > 0) return `${hours}h ${minutes}m`;
  return `${minutes}m`;
}

/** "resets in 2h 5m", or with refill "↻ 2h 5m"; null when the window says nothing of its reset. */
export function formatResetsIn(window: UsageLimitWindow, now: number, style: "full" | "refill" = "full"): string | null {
  const resetsAt = window.resetsAt ? Date.parse(window.resetsAt) : NaN;
  if (Number.isNaN(resetsAt)) return null;
  if (resetsAt <= now) return "Resets momentarily";
  const duration = formatDuration(resetsAt - now);
  return style === "refill" ? `↻ ${duration}` : `resets in ${duration}`;
}

const KIND_ORDER: Record<UsageLimitWindow["kind"], number> = { session: 0, weekly: 1, monthly: 2 };
export const sortLimitWindows = (windows: UsageLimitWindow[]) => windows.toSorted((a, b) => KIND_ORDER[a.kind] - KIND_ORDER[b.kind]);
