// What GET /api/usage and /api/usage/limits answer. Shapes follow T3 Code's usage contract and merged
// usage (packages/contracts/src/usage.ts, packages/shared/src/usageMerge.ts; MIT, see NOTICE), for one machine.

export const USAGE_PROVIDERS = ["codex", "claude"] as const;
export type UsageProvider = (typeof USAGE_PROVIDERS)[number];

/** A window of whole days in the viewer's zone, or the 24 hours up to now in hours. */
export type UsageWindow =
  | { resolution: "day"; sinceDay: string; untilDay: string; timeZone: string }
  | { resolution: "hour"; sinceTime: string; untilTime: string; timeZone: string };

export interface UsageAmount {
  costUsd: number;
  totalTokens: number;
}

export interface UsageProviderTotals extends UsageAmount {
  provider: UsageProvider;
  sessions: number;
  costShare: number;
  tokenShare: number;
}

export interface UsageModelTotals extends UsageAmount {
  provider: UsageProvider;
  model: string;
  records: number;
  unpricedRecords: number;
  costShare: number;
  tokenShare: number;
}

/** One day ("2026-11-26") or one hour (its start, ISO 8601); only periods with usage are listed. */
export interface UsagePeriod extends UsageAmount {
  period: string;
  byProvider: Partial<Record<UsageProvider, UsageAmount>>;
}

export interface UsageReport {
  costUsd: number;
  uncachedInputTokens: number;
  cachedInputTokens: number;
  cacheCreationTokens: number;
  outputTokens: number;
  totalTokens: number;
  sessions: number;
  providers: UsageProviderTotals[];
  models: UsageModelTotals[];
  periods: UsagePeriod[];
  /** Of all records, the share no rate was found for. */
  unpricedShare: number;
  cacheSavingsUsd: number;
  /** unsplit: provider-reported cost that isn't broken down by type. */
  categoryCost: { input: number; cacheRead: number; cacheWrite: number; output: number; unsplit: number };
  /** premium: what fast and ultrafast cost above standard rates. */
  speedCost: { standard: number; fast: number; ultrafast: number; premium: number };
  /** Why the numbers may be incomplete, worded for the user. */
  notices: string[];
}

export interface UsageLimitWindow {
  id: string;
  kind: "session" | "weekly" | "monthly";
  label: string;
  /** 0 to 100. */
  usedPercent: number;
  windowDurationMins?: number;
  /** ISO 8601. */
  resetsAt?: string;
}

/** A provider's subscription limits. */
export interface UsageLimits {
  provider: UsageProvider;
  windows: UsageLimitWindow[];
  /** Why there are none to show, as with an API key or a failed read; null when there are. */
  unavailable: string | null;
}
