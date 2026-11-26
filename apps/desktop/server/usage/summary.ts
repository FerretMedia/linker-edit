// Sums the transcripts' usage over a window, the way T3 Code's server aggregates it and its page merges it
// (apps/server/src/usage/usageAggregation.ts, packages/shared/src/usageMerge.ts; MIT, see NOTICE).
import { USAGE_PROVIDERS, type UsageAmount, type UsageLimits, type UsageProvider, type UsageReport, type UsageWindow } from "../../shared/usage.ts";
import { readLimits } from "./limits.ts";
import { priceUsage, rateLoader, type RateTable } from "./pricing.ts";
import { listTranscripts, readTranscript, totalTokens, transcriptDirs, type UsageRecord } from "./transcripts.ts";

const HOUR_MS = 60 * 60 * 1000;
// A file last written before the window can't hold its records; the slack covers every time zone's midnight.
const MTIME_SLACK_MS = 36 * HOUR_MS;

/** Whether a record falls in the window, and which period it counts in. */
function periodOf(window: UsageWindow): (timestampMs: number) => string | null {
  if (window.resolution === "hour") {
    const since = Date.parse(window.sinceTime);
    const until = Date.parse(window.untilTime);
    // Hours run from the window's start, which needn't be on the hour.
    return (ms) => (ms >= since && ms < until ? new Date(since + Math.floor((ms - since) / HOUR_MS) * HOUR_MS).toISOString() : null);
  }
  let format: Intl.DateTimeFormat;
  try {
    format = new Intl.DateTimeFormat("en-CA", { timeZone: window.timeZone, year: "numeric", month: "2-digit", day: "2-digit" });
  } catch {
    format = new Intl.DateTimeFormat("en-CA", { timeZone: "UTC", year: "numeric", month: "2-digit", day: "2-digit" });
  }
  return (ms) => {
    const day = format.format(new Date(ms));
    return day >= window.sinceDay && day <= window.untilDay ? day : null;
  };
}

const windowStartMs = (window: UsageWindow) =>
  window.resolution === "hour" ? Date.parse(window.sinceTime) : Date.parse(`${window.sinceDay}T00:00:00Z`);

/**
 * Partial copies share a billed message's key: keep each token channel's maximum and the larger reported cost, and
 * the first copy's time, model, session and speed so it stays in its period.
 */
function mergeUsageCopies(held: UsageRecord, copy: UsageRecord): UsageRecord {
  const a = held.totals;
  const b = copy.totals;
  const reportedCostUsd =
    held.reportedCostUsd === null || copy.reportedCostUsd === null
      ? (held.reportedCostUsd ?? copy.reportedCostUsd)
      : Math.max(held.reportedCostUsd, copy.reportedCostUsd);
  const cacheCreation1hTokens = Math.max(held.cacheCreation1hTokens ?? 0, copy.cacheCreation1hTokens ?? 0);
  return {
    ...held,
    ...(cacheCreation1hTokens > 0 ? { cacheCreation1hTokens } : {}),
    totals: {
      uncachedInputTokens: Math.max(a.uncachedInputTokens, b.uncachedInputTokens),
      cachedInputTokens: Math.max(a.cachedInputTokens, b.cachedInputTokens),
      cacheCreationTokens: Math.max(a.cacheCreationTokens, b.cacheCreationTokens),
      outputTokens: Math.max(a.outputTokens, b.outputTokens),
      reasoningTokens: Math.max(a.reasoningTokens, b.reasoningTokens),
    },
    reportedCostUsd,
  };
}

/** Sums records already read; exported for tests. */
export function summarize(records: UsageRecord[], window: UsageWindow, rates: RateTable, notices: string[] = []): UsageReport {
  const periodFor = periodOf(window);
  const unique = new Map<string, UsageRecord>();
  const kept: { record: UsageRecord; period: string }[] = [];
  for (const record of records) {
    if (record.dedupeKey === null) continue;
    const held = unique.get(record.dedupeKey);
    unique.set(record.dedupeKey, held ? mergeUsageCopies(held, record) : record);
  }
  for (const record of [...records.filter((candidate) => candidate.dedupeKey === null), ...unique.values()]) {
    const period = periodFor(record.timestampMs);
    if (period !== null) kept.push({ record, period });
  }

  const report: UsageReport = {
    costUsd: 0,
    uncachedInputTokens: 0,
    cachedInputTokens: 0,
    cacheCreationTokens: 0,
    outputTokens: 0,
    totalTokens: 0,
    sessions: 0,
    providers: [],
    models: [],
    periods: [],
    unpricedShare: 0,
    cacheSavingsUsd: 0,
    categoryCost: { input: 0, cacheRead: 0, cacheWrite: 0, output: 0, unsplit: 0 },
    speedCost: { standard: 0, fast: 0, ultrafast: 0, premium: 0 },
    notices,
  };
  const providers = new Map<UsageProvider, UsageAmount & { sessions: Set<string> }>();
  const models = new Map<string, UsageAmount & { provider: UsageProvider; model: string; records: number; unpricedRecords: number }>();
  const periods = new Map<string, UsageAmount & { byProvider: Partial<Record<UsageProvider, UsageAmount>> }>();
  let unpricedRecords = 0;

  for (const { record, period } of kept) {
    const priced = priceUsage(rates, record);
    const tokens = totalTokens(record.totals);
    report.costUsd += priced.costUsd;
    report.uncachedInputTokens += record.totals.uncachedInputTokens;
    report.cachedInputTokens += record.totals.cachedInputTokens;
    report.cacheCreationTokens += record.totals.cacheCreationTokens;
    report.outputTokens += record.totals.outputTokens;
    report.totalTokens += tokens;
    report.cacheSavingsUsd += priced.cacheSavingsUsd;
    if (priced.costSource === "unpriced") unpricedRecords += 1;
    if (priced.categoryCostUsd) {
      report.categoryCost.input += priced.categoryCostUsd.input;
      report.categoryCost.cacheRead += priced.categoryCostUsd.cacheRead;
      report.categoryCost.cacheWrite += priced.categoryCostUsd.cacheWrite;
      report.categoryCost.output += priced.categoryCostUsd.output;
    }
    if (record.speed !== "standard") report.speedCost[record.speed] += priced.costUsd;
    report.speedCost.premium += priced.speedPremiumUsd;

    const provider = providers.get(record.provider) ?? { costUsd: 0, totalTokens: 0, sessions: new Set<string>() };
    provider.costUsd += priced.costUsd;
    provider.totalTokens += tokens;
    if (record.sessionId) provider.sessions.add(record.sessionId);
    providers.set(record.provider, provider);

    const modelKey = `${record.provider}:${record.model}`;
    const model = models.get(modelKey) ?? { provider: record.provider, model: record.model, costUsd: 0, totalTokens: 0, records: 0, unpricedRecords: 0 };
    model.costUsd += priced.costUsd;
    model.totalTokens += tokens;
    model.records += 1;
    if (priced.costSource === "unpriced") model.unpricedRecords += 1;
    models.set(modelKey, model);

    const entry = periods.get(period) ?? { costUsd: 0, totalTokens: 0, byProvider: {} };
    entry.costUsd += priced.costUsd;
    entry.totalTokens += tokens;
    const byProvider = (entry.byProvider[record.provider] ??= { costUsd: 0, totalTokens: 0 });
    byProvider.costUsd += priced.costUsd;
    byProvider.totalTokens += tokens;
    periods.set(period, entry);
  }

  const share = (part: number, whole: number) => (whole === 0 ? 0 : part / whole);
  report.providers = USAGE_PROVIDERS.flatMap((provider) => {
    const totals = providers.get(provider);
    if (!totals) return [];
    const sessions = totals.sessions.size;
    report.sessions += sessions;
    return [
      {
        provider,
        costUsd: totals.costUsd,
        totalTokens: totals.totalTokens,
        sessions,
        costShare: share(totals.costUsd, report.costUsd),
        tokenShare: share(totals.totalTokens, report.totalTokens),
      },
    ];
  });
  report.models = [...models.values()]
    .map((model) => ({ ...model, costShare: share(model.costUsd, report.costUsd), tokenShare: share(model.totalTokens, report.totalTokens) }))
    .sort((a, b) => b.costUsd - a.costUsd || b.totalTokens - a.totalTokens);
  report.periods = [...periods].map(([period, totals]) => ({ period, ...totals })).sort((a, b) => (a.period < b.period ? -1 : 1));
  report.unpricedShare = share(unpricedRecords, kept.length);
  const { input, cacheRead, cacheWrite, output } = report.categoryCost;
  report.categoryCost.unsplit = Math.max(0, report.costUsd - input - cacheRead - cacheWrite - output);
  report.speedCost.standard = Math.max(0, report.costUsd - report.speedCost.fast - report.speedCost.ultrafast);
  return report;
}

export function formatMalformedUsageWarning(count: number): string {
  return `${count} malformed or inconsistent usage record(s) were skipped; usage may be incomplete.`;
}

/**
 * Reads usage for a window from the transcripts in dirs. Each file's records are kept while its size and time stay
 * the same, so a refresh only rereads the transcripts that grew.
 */
export function usageReader(dirs: Record<UsageProvider, string>, rates: () => Promise<RateTable>) {
  const files = new Map<string, { size: number; mtimeMs: number; records: UsageRecord[]; malformed: number }>();

  async function readProvider(provider: UsageProvider, sinceMs: number, notices: string[]) {
    let listed;
    try {
      listed = await listTranscripts(dirs[provider], sinceMs);
    } catch (error) {
      notices.push(`Could not read ${dirs[provider]}: ${error instanceof Error ? error.message : String(error)}`);
      return { records: [], malformed: 0 };
    }
    const records: UsageRecord[] = [];
    let malformed = 0;
    // A few at a time: a heavy user has thousands of transcripts.
    const queue = (listed ?? []).values();
    await Promise.all(
      Array.from({ length: 8 }, async () => {
        for (const file of queue) {
          let read = files.get(file.path);
          if (!read || read.size !== file.size || read.mtimeMs !== file.mtimeMs) {
            try {
              read = { ...file, ...(await readTranscript(provider, file.path)) };
              files.set(file.path, read);
            } catch {
              // Deleted or locked since it was listed; the next refresh tries again.
              continue;
            }
          }
          for (const record of read.records) records.push(record);
          malformed += read.malformed;
        }
      }),
    );
    return { records, malformed };
  }

  return async function read(window: UsageWindow, providers: readonly UsageProvider[] = USAGE_PROVIDERS): Promise<UsageReport> {
    const sinceMs = windowStartMs(window) - MTIME_SLACK_MS;
    const notices: string[] = [];
    const [table, ...read] = await Promise.all([rates(), ...providers.map((provider) => readProvider(provider, sinceMs, notices))]);
    const malformed = read.reduce((sum, entry) => sum + entry.malformed, 0);
    if (malformed > 0) notices.push(formatMalformedUsageWarning(malformed));
    return summarize(
      read.flatMap((entry) => entry.records),
      window,
      table,
      notices,
    );
  };
}

/** The window GET /api/usage asks for: since and until (days), or from and to (hours), and tz. */
export function parseUsageWindow(query: URLSearchParams): UsageWindow | null {
  const timeZone = query.get("tz") || "UTC";
  const from = query.get("from");
  const to = query.get("to");
  if (from && to) {
    const sinceMs = Date.parse(from);
    const untilMs = Date.parse(to);
    // At most a day of hours, as the page asks.
    if (Number.isNaN(sinceMs) || Number.isNaN(untilMs) || untilMs <= sinceMs || untilMs - sinceMs > 24 * HOUR_MS) return null;
    return { resolution: "hour", sinceTime: new Date(sinceMs).toISOString(), untilTime: new Date(untilMs).toISOString(), timeZone };
  }
  const sinceDay = query.get("since") ?? "";
  const untilDay = query.get("until") ?? "";
  const day = /^\d{4}-\d{2}-\d{2}$/;
  return day.test(sinceDay) && day.test(untilDay) && sinceDay <= untilDay ? { resolution: "day", sinceDay, untilDay, timeZone } : null;
}

/** The providers GET /api/usage asks for with provider=…; every one when it names none. */
export function parseUsageProviders(query: URLSearchParams): UsageProvider[] {
  const asked = query.getAll("provider");
  return asked.length === 0 ? [...USAGE_PROVIDERS] : USAGE_PROVIDERS.filter((provider) => asked.includes(provider));
}

export interface UsageService {
  read(window: UsageWindow, providers?: readonly UsageProvider[]): Promise<UsageReport>;
  limits(): Promise<UsageLimits[]>;
}

/** Usage from this computer's Claude Code and Codex transcripts, priced with rates kept in ratesFile. */
export function createUsage({ ratesFile, version }: { ratesFile: string; version: string }): UsageService {
  return { read: usageReader(transcriptDirs(), rateLoader(ratesFile)), limits: () => readLimits(version) };
}
