// API-equivalent prices from LiteLLM's public rate table, as T3 Code prices usage
// (apps/server/src/usage/usagePricing.ts and UsageService.ts; MIT, see NOTICE).
import * as fs from "node:fs/promises";
import * as path from "node:path";
import type { TokenTotals, UsageRecord, UsageSpeed } from "./transcripts.ts";

interface TokenRates {
  inputCostPerToken: number;
  outputCostPerToken: number;
  cacheReadCostPerToken: number;
  cacheCreationCostPerToken: number;
  /** Cache writes with a 1-hour TTL; the 5-minute rate when unpublished. */
  cacheCreation1hCostPerToken: number;
}

interface SpeedRates extends TokenRates {
  fast: TokenRates | null;
  ultrafast: TokenRates | null;
}

/** A request whose prompt exceeds thresholdTokens bills every token at this tier. */
interface ContextTier {
  thresholdTokens: number;
  rate: SpeedRates;
}

interface ModelRate extends SpeedRates {
  /** Sorted by descending threshold. */
  contextTiers: ContextTier[];
}

export type RateTable = ReadonlyMap<string, ModelRate>;

type LiteLlmEntry = Readonly<Record<string, unknown>>;

const finiteNumber = (value: unknown) => (typeof value === "number" && Number.isFinite(value) ? value : null);

// Missing cache rates fall back to input pricing, and a missing 1-hour write rate to the 5-minute one.
function readTokenRates(entry: LiteLlmEntry, suffix: string, standard?: TokenRates, outputFallback?: number): TokenRates | null {
  const input = finiteNumber(entry[`input_cost_per_token${suffix}`]);
  const output = finiteNumber(entry[`output_cost_per_token${suffix}`]) ?? outputFallback ?? null;
  if (input === null || output === null) return null;
  const cacheRate = (name: string, field: "cacheReadCostPerToken" | "cacheCreationCostPerToken" | "cacheCreation1hCostPerToken", fallback = input) =>
    finiteNumber(entry[`${name}${suffix}`]) ??
    (standard !== undefined && standard.inputCostPerToken > 0 ? (standard[field] / standard.inputCostPerToken) * input : fallback);
  const cacheCreationCostPerToken = cacheRate("cache_creation_input_token_cost", "cacheCreationCostPerToken");
  return {
    inputCostPerToken: input,
    outputCostPerToken: output,
    cacheReadCostPerToken: cacheRate("cache_read_input_token_cost", "cacheReadCostPerToken"),
    cacheCreationCostPerToken,
    cacheCreation1hCostPerToken: cacheRate("cache_creation_input_token_cost_above_1hr", "cacheCreation1hCostPerToken", cacheCreationCostPerToken),
  };
}

function scaleTokenRates(rates: TokenRates, multiple: number): TokenRates {
  return {
    inputCostPerToken: rates.inputCostPerToken * multiple,
    outputCostPerToken: rates.outputCostPerToken * multiple,
    cacheReadCostPerToken: rates.cacheReadCostPerToken * multiple,
    cacheCreationCostPerToken: rates.cacheCreationCostPerToken * multiple,
    cacheCreation1hCostPerToken: rates.cacheCreation1hCostPerToken * multiple,
  };
}

function fastMultiplier(entry: LiteLlmEntry): number | null {
  const specific = entry.provider_specific_entry;
  if (typeof specific !== "object" || specific === null) return null;
  const fast = finiteNumber((specific as Record<string, unknown>).fast);
  return fast !== null && fast > 0 ? fast : null;
}

function readSpeedRates(entry: LiteLlmEntry, standard: TokenRates, suffix = ""): SpeedRates {
  const multiple = fastMultiplier(entry);
  return {
    ...standard,
    fast: multiple === null ? readTokenRates(entry, `${suffix}_priority`, standard) : scaleTokenRates(standard, multiple),
    ultrafast: readTokenRates(entry, `${suffix}_ultrafast`, standard),
  };
}

const CONTEXT_TIER_INPUT_KEY = /^input_cost_per_token_above_(\d{1,7})k_tokens(?:_priority|_ultrafast)?$/;

// Fields a tier omits fall back to base rates, so a priority-only tier still prices standard requests at base.
function readContextTiers(entry: LiteLlmEntry, base: TokenRates): ContextTier[] {
  const thresholds = new Set<string>();
  for (const key of Object.keys(entry)) {
    const match = CONTEXT_TIER_INPUT_KEY.exec(key);
    if (match !== null) thresholds.add(match[1]!);
  }
  return [...thresholds]
    .map((threshold) => {
      const suffix = `_above_${threshold}k_tokens`;
      const standard = readTokenRates(entry, suffix, base, base.outputCostPerToken) ?? base;
      return { thresholdTokens: Number(threshold) * 1_000, rate: readSpeedRates(entry, standard, suffix) };
    })
    .sort((a, b) => b.thresholdTokens - a.thresholdTokens);
}

// LiteLLM prefixes for the model makers' own APIs; their price beats a cloud's or reseller's for a bare name.
const FIRST_PARTY_PROVIDERS = new Set(["anthropic", "deepseek", "gemini", "meta", "mistral", "openai", "xai"]);

const normalizeRateKey = (model: string) => model.trim().toLowerCase();

function bareModelName(key: string): string {
  const slash = key.lastIndexOf("/");
  return slash === -1 ? key : key.slice(slash + 1);
}

// Only Bedrock's base ID gives a bare alias; its regional and GovCloud IDs carry markups.
const bedrockBaseModelName = (key: string) => (key.startsWith("anthropic.") ? key.slice("anthropic.".length) : key);

function sameTokenRates(a: TokenRates | null, b: TokenRates | null): boolean {
  if (a === null || b === null) return a === b;
  return (
    a.inputCostPerToken === b.inputCostPerToken &&
    a.outputCostPerToken === b.outputCostPerToken &&
    a.cacheReadCostPerToken === b.cacheReadCostPerToken &&
    a.cacheCreationCostPerToken === b.cacheCreationCostPerToken &&
    a.cacheCreation1hCostPerToken === b.cacheCreation1hCostPerToken
  );
}

// Resellers often omit speed and context tiers, so a missing tier fills from the other entry instead of conflicting.
function mergeSpeedRates(a: SpeedRates, b: SpeedRates): SpeedRates | null {
  const fast = a.fast ?? b.fast;
  const ultrafast = a.ultrafast ?? b.ultrafast;
  if (!sameTokenRates(a, b) || !sameTokenRates(fast, b.fast ?? fast) || !sameTokenRates(ultrafast, b.ultrafast ?? ultrafast)) return null;
  return { ...a, fast, ultrafast };
}

function mergeContextTiers(a: ContextTier[], b: ContextTier[]): ContextTier[] | null {
  if (a.length === 0) return b;
  if (b.length === 0) return a;
  if (a.length !== b.length) return null;
  const merged: ContextTier[] = [];
  for (const [index, tier] of a.entries()) {
    const other = b[index]!;
    const rate = mergeSpeedRates(tier.rate, other.rate);
    if (tier.thresholdTokens !== other.thresholdTokens || rate === null) return null;
    merged.push({ thresholdTokens: tier.thresholdTokens, rate });
  }
  return merged;
}

function mergeAliasRate(a: ModelRate, b: ModelRate): ModelRate | null {
  const speeds = mergeSpeedRates(a, b);
  const contextTiers = mergeContextTiers(a.contextTiers, b.contextTiers);
  return speeds === null || contextTiers === null ? null : { ...speeds, contextTiers };
}

/** Drops models missing input or output rates to avoid understating cost. */
export function parseRateTable(document: unknown): RateTable {
  const table = new Map<string, ModelRate>();
  if (typeof document !== "object" || document === null) return table;

  for (const [name, raw] of Object.entries(document as Record<string, unknown>)) {
    if (typeof raw !== "object" || raw === null) continue;
    const entry = raw as LiteLlmEntry;
    const standard = readTokenRates(entry, "");
    if (standard === null) continue;
    const key = normalizeRateKey(name);
    if (key.length === 0) continue;
    table.set(key, { ...readSpeedRates(entry, standard), contextTiers: readContextTiers(entry, standard) });
  }

  const aliasCandidates = new Map<string, { rate: ModelRate | null; firstParty: boolean }>();
  for (const [key, rate] of table) {
    const slash = key.indexOf("/");
    const alias = slash === -1 ? bedrockBaseModelName(key) : bareModelName(key);
    if (alias.length === 0 || alias === key || table.has(alias)) continue;
    const firstParty = slash !== -1 && FIRST_PARTY_PROVIDERS.has(key.slice(0, slash));
    const held = aliasCandidates.get(alias);
    if (held === undefined || (firstParty && !held.firstParty)) aliasCandidates.set(alias, { rate, firstParty });
    else if (held.rate !== null && firstParty === held.firstParty) aliasCandidates.set(alias, { rate: mergeAliasRate(held.rate, rate), firstParty });
  }
  for (const [alias, { rate }] of aliasCandidates) {
    if (rate !== null) table.set(alias, rate);
  }
  return table;
}

const promptTokens = (totals: TokenTotals) => totals.uncachedInputTokens + totals.cachedInputTokens + totals.cacheCreationTokens;

// A tier without the requested speed keeps the base tier's rate for that speed.
function ratesAt(rate: ModelRate, speed: UsageSpeed, totals: TokenTotals): TokenRates {
  const prompt = promptTokens(totals);
  const tier = rate.contextTiers.find((candidate) => prompt > candidate.thresholdTokens)?.rate;
  if (speed === "standard") return tier ?? rate;
  return tier?.[speed] ?? rate[speed] ?? tier ?? rate;
}

// Price Claude's bracketed context variants at the base tier, the only name the rate table knows.
function stripVariantSuffix(key: string): string {
  const bracket = key.indexOf("[");
  return bracket === -1 ? key : key.slice(0, bracket);
}

// Synthetic messages are unbilled; bare family names are generation-ambiguous and remain unpriced.
const UNPRICEABLE_MODELS = new Set(["<synthetic>", "synthetic", "opus", "sonnet", "haiku", "fable"]);

function lookupRate(table: RateTable, model: string): ModelRate | null {
  const key = stripVariantSuffix(normalizeRateKey(model));
  const bareName = bareModelName(key);
  if (bareName.length === 0 || UNPRICEABLE_MODELS.has(bareName)) return null;
  return table.get(key) ?? null;
}

export interface CategoryCost {
  input: number;
  cacheRead: number;
  cacheWrite: number;
  output: number;
}

export interface PricedUsage {
  costUsd: number;
  costSource: "providerReported" | "modelPriced" | "unpriced";
  categoryCostUsd: CategoryCost | null;
  speedPremiumUsd: number;
  cacheSavingsUsd: number;
}

function costByCategory(record: UsageRecord, rates: TokenRates): CategoryCost {
  const { totals } = record;
  const oneHour = Math.min(record.cacheCreation1hTokens ?? 0, totals.cacheCreationTokens);
  return {
    input: totals.uncachedInputTokens * rates.inputCostPerToken,
    cacheRead: totals.cachedInputTokens * rates.cacheReadCostPerToken,
    cacheWrite: (totals.cacheCreationTokens - oneHour) * rates.cacheCreationCostPerToken + oneHour * rates.cacheCreation1hCostPerToken,
    output: totals.outputTokens * rates.outputCostPerToken,
  };
}

const sumCategories = (cost: CategoryCost) => cost.input + cost.cacheRead + cost.cacheWrite + cost.output;

/** Reasoning tokens are already included in outputTokens; they aren't charged twice. */
export function priceUsage(table: RateTable, record: UsageRecord): PricedUsage {
  const reported = record.reportedCostUsd !== null && Number.isFinite(record.reportedCostUsd) ? record.reportedCostUsd : null;
  const rate = lookupRate(table, record.model);
  const unsplit = (costUsd: number, costSource: PricedUsage["costSource"]): PricedUsage => ({
    costUsd,
    costSource,
    categoryCostUsd: null,
    speedPremiumUsd: 0,
    cacheSavingsUsd: 0,
  });
  if (rate === null) return reported === null ? unsplit(0, "unpriced") : unsplit(reported, "providerReported");

  const rates = ratesAt(rate, record.speed, record.totals);
  const cacheSavingsUsd = record.totals.cachedInputTokens * (rates.inputCostPerToken - rates.cacheReadCostPerToken);
  const listCost = costByCategory(record, rates);
  const listCostUsd = sumCategories(listCost);
  if (reported !== null && listCostUsd <= 0) return { ...unsplit(reported, "providerReported"), cacheSavingsUsd };
  const premiumUsd = record.speed === "standard" ? 0 : listCostUsd - sumCategories(costByCategory(record, ratesAt(rate, "standard", record.totals)));
  const scale = reported === null ? 1 : reported / listCostUsd;
  return {
    costUsd: reported ?? listCostUsd,
    costSource: reported === null ? "modelPriced" : "providerReported",
    categoryCostUsd: {
      input: listCost.input * scale,
      cacheRead: listCost.cacheRead * scale,
      cacheWrite: listCost.cacheWrite * scale,
      output: listCost.output * scale,
    },
    speedPremiumUsd: premiumUsd * scale,
    cacheSavingsUsd,
  };
}

const LITELLM_RATES_URL = "https://raw.githubusercontent.com/BerriAI/litellm/main/model_prices_and_context_window.json";
const RATES_MAX_AGE_MS = 24 * 60 * 60 * 1000;

/**
 * The rate table, fetched at most once a day and kept in cacheFile so usage still prices offline. An empty table
 * leaves every record unpriced, which the page says.
 */
export function rateLoader(cacheFile: string, fetchFn: typeof fetch = fetch, now = Date.now) {
  let rates: RateTable | null = null;
  let fetchedAtMs = 0;
  // Offline, a minute passes before the next try, so each refresh isn't held up by a fetch that fails.
  let triedAtMs = -Infinity;
  let loading: Promise<RateTable> | null = null;

  async function load(): Promise<RateTable> {
    if (rates === null) {
      try {
        const saved = JSON.parse(await fs.readFile(cacheFile, "utf8")) as { fetchedAtMs: number; document: unknown };
        rates = parseRateTable(saved.document);
        fetchedAtMs = saved.fetchedAtMs;
      } catch {
        rates = new Map();
      }
    }
    if (now() - fetchedAtMs < RATES_MAX_AGE_MS || now() - triedAtMs < 60_000) return rates;
    triedAtMs = now();
    try {
      const res = await fetchFn(LITELLM_RATES_URL, { signal: AbortSignal.timeout(15_000) });
      if (!res.ok) throw new Error(`HTTP ${res.status}`);
      const document: unknown = await res.json();
      const parsed = parseRateTable(document);
      if (parsed.size === 0) throw new Error("empty rate table");
      rates = parsed;
      fetchedAtMs = now();
      await fs.mkdir(path.dirname(cacheFile), { recursive: true });
      await fs.writeFile(cacheFile, JSON.stringify({ fetchedAtMs, document }));
    } catch (error) {
      // The saved table, if any, still prices.
      console.warn(`Could not fetch model prices: ${error instanceof Error ? error.message : String(error)}`);
    }
    return rates;
  }

  return () => (loading ??= load().finally(() => (loading = null)));
}
