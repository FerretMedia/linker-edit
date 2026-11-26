// Reads token usage from the transcripts Claude Code and Codex keep on disk, as T3 Code does
// (apps/server/src/provider/Drivers/claudeUsage.ts, codexUsage.ts and usage/usageTranscriptReader.ts;
// MIT, see NOTICE). Linker-Edit's own chats are among them: both agents write their transcripts there.
import * as fs from "node:fs/promises";
import * as os from "node:os";
import * as path from "node:path";
import type { UsageProvider } from "../../shared/usage.ts";

export interface TokenTotals {
  uncachedInputTokens: number;
  cachedInputTokens: number;
  cacheCreationTokens: number;
  outputTokens: number;
  /** A part of outputTokens, never added to it again. */
  reasoningTokens: number;
}

export type UsageSpeed = "standard" | "fast" | "ultrafast";

export interface UsageRecord {
  provider: UsageProvider;
  timestampMs: number;
  model: string;
  sessionId: string;
  totals: TokenTotals;
  /** The part of totals.cacheCreationTokens written with a 1-hour TTL. */
  cacheCreation1hTokens?: number;
  reportedCostUsd: number | null;
  speed: UsageSpeed;
  /** Copies of one billed message share it; null when every record stands alone. */
  dedupeKey: string | null;
}

export const totalTokens = (totals: TokenTotals) =>
  totals.uncachedInputTokens + totals.cachedInputTokens + totals.cacheCreationTokens + totals.outputTokens;

function tokenCount(value: unknown): number {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? Math.trunc(value) : 0;
}

function parseTimestampMs(value: unknown): number | null {
  if (typeof value !== "string") return null;
  const parsed = Date.parse(value);
  return Number.isNaN(parsed) ? null : parsed;
}

type Json = Record<string, unknown>;
const isObject = (value: unknown): value is Json => typeof value === "object" && value !== null && !Array.isArray(value);

// ---- Claude Code ----

// Claude Code uses this model for local placeholders that never reached the API.
const CLAUDE_SYNTHETIC_MODEL = "<synthetic>";

function cacheCreation1h(usage: Json): { cacheCreation1hTokens?: number } {
  const split = usage.cache_creation;
  if (!isObject(split)) return {};
  const tokens = tokenCount(split.ephemeral_1h_input_tokens);
  return tokens > 0 ? { cacheCreation1hTokens: tokens } : {};
}

/**
 * Claude Code repeats a message's dedupeKey for each assistant content block. Subagent blocks can progress from
 * stream start to final usage, so each key counts once, with the maximum of each token channel.
 */
export function parseClaudeLine(line: string): UsageRecord[] {
  let record: unknown;
  try {
    record = JSON.parse(line);
  } catch {
    return [];
  }
  if (!isObject(record) || record.type !== "assistant") return [];
  const message = record.message;
  if (!isObject(message) || !isObject(message.usage)) return [];
  const usage = message.usage;
  const timestampMs = parseTimestampMs(record.timestamp);
  if (timestampMs === null) return [];
  const model = typeof message.model === "string" ? message.model : "";
  if (model.length === 0 || model === CLAUDE_SYNTHETIC_MODEL) return [];

  const messageId = typeof message.id === "string" ? message.id : null;
  const requestId = typeof record.requestId === "string" ? record.requestId : null;
  const dedupeKey = messageId === null && requestId === null ? null : `${messageId ?? ""}:${requestId ?? ""}`;
  const cost = record.costUSD;

  const executor: UsageRecord = {
    provider: "claude",
    timestampMs,
    model,
    sessionId: typeof record.sessionId === "string" ? record.sessionId : "",
    totals: {
      uncachedInputTokens: tokenCount(usage.input_tokens),
      cachedInputTokens: tokenCount(usage.cache_read_input_tokens),
      cacheCreationTokens: tokenCount(usage.cache_creation_input_tokens),
      outputTokens: tokenCount(usage.output_tokens),
      // Anthropic folds thinking tokens into output and does not break them out.
      reasoningTokens: 0,
    },
    ...cacheCreation1h(usage),
    reportedCostUsd: typeof cost === "number" && Number.isFinite(cost) ? cost : null,
    speed: usage.speed === "fast" ? "fast" : "standard",
    dedupeKey,
  };

  // Top-level usage is the executor's alone. Advisor calls run on another model at its own rates and appear only
  // in iterations, so each one is its own record under that model.
  const records = [executor];
  const iterations = Array.isArray(usage.iterations) ? (usage.iterations as unknown[]) : [];
  for (const [index, advisor] of iterations.entries()) {
    if (!isObject(advisor) || advisor.type !== "advisor_message") continue;
    const advisorModel = typeof advisor.model === "string" ? advisor.model : "";
    if (advisorModel.length === 0 || advisorModel === CLAUDE_SYNTHETIC_MODEL) continue;
    records.push({
      provider: "claude",
      timestampMs,
      model: advisorModel,
      sessionId: executor.sessionId,
      totals: {
        uncachedInputTokens: tokenCount(advisor.input_tokens),
        cachedInputTokens: tokenCount(advisor.cache_read_input_tokens),
        cacheCreationTokens: tokenCount(advisor.cache_creation_input_tokens),
        outputTokens: tokenCount(advisor.output_tokens),
        reasoningTokens: 0,
      },
      ...cacheCreation1h(advisor),
      reportedCostUsd: null,
      speed: executor.speed,
      dedupeKey: dedupeKey === null ? null : `${dedupeKey}:advisor:${index}`,
    });
  }
  return records;
}

// ---- Codex ----

interface CodexTokenUsage {
  input_tokens: number;
  cached_input_tokens: number;
  cache_write_input_tokens: number;
  output_tokens: number;
  reasoning_output_tokens: number;
  total_tokens: number;
}
const CODEX_USAGE_FIELDS: (keyof CodexTokenUsage)[] = [
  "input_tokens",
  "cached_input_tokens",
  "cache_write_input_tokens",
  "output_tokens",
  "reasoning_output_tokens",
  "total_tokens",
];

const isTokenCount = (value: unknown) => typeof value === "number" && Number.isInteger(value) && value >= 0;

function readCodexTokenUsage(value: unknown): CodexTokenUsage | null {
  if (!isObject(value)) return null;
  const input = value.input_tokens;
  const output = value.output_tokens;
  if (typeof input !== "number" || typeof output !== "number") return null;
  const usage = {
    input_tokens: input,
    cached_input_tokens: value.cached_input_tokens ?? 0,
    cache_write_input_tokens: value.cache_write_input_tokens ?? 0,
    output_tokens: output,
    reasoning_output_tokens: value.reasoning_output_tokens ?? 0,
    total_tokens: value.total_tokens ?? input + output,
  };
  if (!CODEX_USAGE_FIELDS.every((key) => isTokenCount(usage[key]))) return null;
  const valid = usage as CodexTokenUsage;
  return valid.cached_input_tokens + valid.cache_write_input_tokens <= valid.input_tokens &&
    valid.reasoning_output_tokens <= valid.output_tokens &&
    valid.total_tokens >= valid.input_tokens + valid.output_tokens
    ? valid
    : null;
}

const combineCodexUsage = (a: CodexTokenUsage | null, b: CodexTokenUsage, sign: 1 | -1) =>
  readCodexTokenUsage(Object.fromEntries(CODEX_USAGE_FIELDS.map((key) => [key, (a?.[key] ?? 0) + sign * b[key]])));

interface CodexRequestAttribution {
  timestampMs: number;
  model: string;
  speed: UsageSpeed;
}
interface CodexUnreportedRequest extends CodexRequestAttribution {
  usage: CodexTokenUsage;
}

/**
 * Codex token_count events carry no model or service tier, so both are carried forward: the model from the most
 * recent turn_context, the tier from the most recent thread_settings_applied.
 */
export interface CodexScanState {
  model: string;
  speed: UsageSpeed;
  sessionId: string;
  lastUsageSignature: string | null;
  lastCumulativeUsage: CodexTokenUsage | null;
  sequenceStartSignature?: string;
  latestCumulativeAtMs?: number;
  /** token_usage_record usage since the last cumulative snapshot, one entry per record. */
  unreportedRequests?: CodexUnreportedRequest[];
  requestLedgerActive?: boolean;
  malformedRecords: number;
  sawSessionMeta: boolean;
  suppressingForkCopies: boolean;
  forkCopyAnchorMs: number;
}

export function initialCodexScanState(): CodexScanState {
  return {
    model: "",
    speed: "standard",
    sessionId: "",
    lastUsageSignature: null,
    lastCumulativeUsage: null,
    malformedRecords: 0,
    sawSessionMeta: false,
    suppressingForkCopies: false,
    forkCopyAnchorMs: 0,
  };
}

// Every move of the baseline dates it, so an older snapshot re-emitted later is never a rewind.
function dateCodexBaseline(state: CodexScanState, timestampMs: number) {
  state.latestCumulativeAtMs = Math.max(state.latestCumulativeAtMs ?? timestampMs, timestampMs);
}

// A zeroed baseline rebases the counters, so the sequence start measured against it is forgotten.
function rebaseCodexUsage(state: CodexScanState, cumulative: CodexTokenUsage, timestampMs: number) {
  state.lastCumulativeUsage = cumulative;
  dateCodexBaseline(state, timestampMs);
  delete state.unreportedRequests;
  if (cumulative.input_tokens + cumulative.output_tokens === 0) delete state.sequenceStartSignature;
}

// An interrupted turn can end before a snapshot reports its last request. Bill those requests only when the
// token_usage_records since the previous snapshot explain the whole gap; last is the final one.
function unreportedRequests(
  previous: CodexTokenUsage | null,
  cumulative: CodexTokenUsage,
  last: CodexTokenUsage,
  pending: CodexUnreportedRequest[] | undefined,
): CodexUnreportedRequest[] | null {
  if (
    previous === null ||
    pending === undefined ||
    CODEX_USAGE_FIELDS.some((key) => cumulative[key] - previous[key] !== pending.reduce((sum, request) => sum + request.usage[key], 0))
  ) {
    return null;
  }
  const final = pending.at(-1)!;
  const remainder = combineCodexUsage(final.usage, last, -1);
  if (remainder === null) return null;
  return [...pending.slice(0, -1), { ...final, usage: remainder }].filter((request) => request.usage.input_tokens + request.usage.output_tokens > 0);
}

// A rewound snapshot vouches only for the request it reports, so the anonymous requests read before it can be
// neither billed nor ruled out as that request's prefix.
function discardedRequests(pending: CodexUnreportedRequest[] | undefined, last: CodexTokenUsage) {
  const final = pending?.at(-1);
  if (pending === undefined || final === undefined) return 0;
  const reported = CODEX_USAGE_FIELDS.every((key) => final.usage[key] === last[key]);
  return pending.length - (reported ? 1 : 0);
}

/** Forks copy and restamp parent history in a burst (0-40ms gaps); genuine child usage follows a model turn (5s+). */
const FORK_COPY_MAX_GAP_MS = 1000;

function isForkedSessionMeta(payload: Json): boolean {
  if (typeof payload.forked_from_id === "string") return true;
  const source = payload.source;
  const subagent = isObject(source) ? source.subagent : undefined;
  const spawn = isObject(subagent) ? subagent.thread_spawn : undefined;
  return isObject(spawn) && typeof spawn.parent_thread_id === "string";
}

// The first usage line past the copied burst ends suppression for the rest of the rollout.
function isForkCopy(state: CodexScanState, timestampMs: number): boolean {
  if (!state.suppressingForkCopies) return false;
  if (timestampMs - state.forkCopyAnchorMs < FORK_COPY_MAX_GAP_MS) {
    state.forkCopyAnchorMs = timestampMs;
    return true;
  }
  state.suppressingForkCopies = false;
  state.lastUsageSignature = null;
  return false;
}

/** Missing, default and standard tiers bill as standard; fast aliases priority. */
function codexSpeed(serviceTier: unknown): UsageSpeed {
  if (serviceTier === "priority" || serviceTier === "fast") return "fast";
  if (serviceTier === "ultrafast") return "ultrafast";
  return "standard";
}

const codexAttribution = (state: CodexScanState, timestampMs: number): CodexRequestAttribution => ({
  timestampMs,
  model: state.model,
  speed: state.speed,
});

function codexRequestRecord(state: CodexScanState, usage: CodexTokenUsage, { timestampMs, model, speed }: CodexRequestAttribution): UsageRecord {
  return {
    provider: "codex",
    timestampMs,
    model,
    sessionId: state.sessionId,
    totals: {
      // Codex reports input_tokens inclusive of the cached portion.
      uncachedInputTokens: usage.input_tokens - usage.cached_input_tokens - usage.cache_write_input_tokens,
      cachedInputTokens: usage.cached_input_tokens,
      cacheCreationTokens: usage.cache_write_input_tokens,
      outputTokens: usage.output_tokens,
      reasoningTokens: usage.reasoning_output_tokens,
    },
    reportedCostUsd: null,
    speed,
    dedupeKey: null,
  };
}

/**
 * Identified request-ledger entries report spending independently of context snapshots. Legacy snapshots validate
 * request deltas against cumulative counters.
 */
export function parseCodexLine(line: string, state: CodexScanState): UsageRecord[] {
  let record: unknown;
  try {
    record = JSON.parse(line);
  } catch {
    return [];
  }
  if (!isObject(record) || !isObject(record.payload)) return [];
  const payload = record.payload;

  if (record.type === "session_meta") {
    // Only the first meta identifies this session; copied ancestor metas would misattribute later records.
    if (state.sawSessionMeta) return [];
    state.sawSessionMeta = true;
    const id = payload.id ?? payload.session_id;
    if (typeof id === "string") state.sessionId = id;
    const metaTimestampMs = parseTimestampMs(record.timestamp);
    if (metaTimestampMs !== null && isForkedSessionMeta(payload)) {
      state.suppressingForkCopies = true;
      state.forkCopyAnchorMs = metaTimestampMs;
    }
    return [];
  }

  if (record.type === "turn_context") {
    state.requestLedgerActive = false;
    if (typeof payload.model === "string") state.model = payload.model;
    return [];
  }

  // Request ledgers include spending that context snapshots can omit.
  if (record.type === "token_usage_record") {
    const timestampMs = parseTimestampMs(record.timestamp);
    const usage = readCodexTokenUsage(payload.usage);
    if (timestampMs === null || usage === null || state.model.length === 0) return [];
    // Copied history is already billed in the parent.
    if (isForkCopy(state, timestampMs)) return [];
    const responseId = payload.response_id;
    if (typeof responseId === "string" && responseId.length > 0) {
      const pending = state.unreportedRequests ?? [];
      state.requestLedgerActive = true;
      for (const request of pending) state.lastCumulativeUsage = combineCodexUsage(state.lastCumulativeUsage, request.usage, 1);
      state.lastCumulativeUsage = combineCodexUsage(state.lastCumulativeUsage, usage, 1);
      dateCodexBaseline(state, timestampMs);
      delete state.unreportedRequests;
      return [
        ...pending.map((request) => codexRequestRecord(state, request.usage, request)),
        { ...codexRequestRecord(state, usage, codexAttribution(state, timestampMs)), dedupeKey: JSON.stringify(["codex", state.sessionId, responseId]) },
      ];
    }
    state.requestLedgerActive = false;
    (state.unreportedRequests ??= []).push({ ...codexAttribution(state, timestampMs), usage });
    return [];
  }

  if (payload.type === "thread_settings_applied") {
    if (isObject(payload.thread_settings)) state.speed = codexSpeed(payload.thread_settings.service_tier);
    return [];
  }

  if (payload.type !== "token_count" || !isObject(payload.info)) return [];
  const info = payload.info;

  // Codex may re-emit these counters after the model or timestamp becomes available.
  const timestampMs = parseTimestampMs(record.timestamp);
  if (timestampMs === null || state.model.length === 0) return [];

  const last = readCodexTokenUsage(info.last_token_usage);
  const rawCumulative = info.total_token_usage;
  const hasCumulative = rawCumulative !== undefined && rawCumulative !== null;
  const cumulative = hasCumulative ? readCodexTokenUsage(rawCumulative) : null;

  if (isForkCopy(state, timestampMs)) {
    // Copied history is already billed in the parent, but establishes the child's baseline.
    const signature = last === null ? null : JSON.stringify(last);
    if (cumulative !== null) rebaseCodexUsage(state, cumulative, timestampMs);
    else if (!hasCumulative && last !== null && signature !== state.lastUsageSignature) {
      state.lastCumulativeUsage = combineCodexUsage(state.lastCumulativeUsage, last, 1);
      dateCodexBaseline(state, timestampMs);
    }
    state.lastUsageSignature = signature;
    return [];
  }

  // Context snapshots can rewind on resume and omit compaction spending; identified requests are authoritative.
  if (state.requestLedgerActive) {
    if (cumulative !== null) rebaseCodexUsage(state, cumulative, timestampMs);
    return [];
  }

  const previous = state.lastCumulativeUsage;
  const latestCumulativeAtMs = state.latestCumulativeAtMs;
  const pending = state.unreportedRequests;
  // A valid snapshot remains the next baseline even when its paired request is malformed.
  if (cumulative !== null) rebaseCodexUsage(state, cumulative, timestampMs);
  if (last === null || (hasCumulative && cumulative === null)) {
    state.malformedRecords += 1;
    return [];
  }

  // Zero-component counters report context size, not a request.
  if (last.input_tokens + last.output_tokens === 0) {
    state.lastUsageSignature = null;
    return [];
  }

  if (cumulative !== null && previous !== null && CODEX_USAGE_FIELDS.every((key) => cumulative[key] === previous[key])) return [];

  if (last.total_tokens !== last.input_tokens + last.output_tokens) {
    state.malformedRecords += 1;
    return [];
  }

  const signature = JSON.stringify(last);
  let unreported: CodexUnreportedRequest[] = [];
  if (cumulative !== null) {
    // A session resumed in a new Codex process restarts its cumulative counters at this request.
    const restarted = CODEX_USAGE_FIELDS.every((key) => cumulative[key] === last[key]);
    const sequenceStart = JSON.stringify([timestampMs, last]);
    // Only the same event is a replay; a later request can restart with identical counters.
    if (restarted && sequenceStart === state.sequenceStartSignature) {
      state.lastCumulativeUsage = previous;
      if (latestCumulativeAtMs === undefined) delete state.latestCumulativeAtMs;
      return [];
    }
    // Forks and continuations can inherit totals without a counter snapshot, and a later request can continue from
    // an earlier snapshot. Only bill the latest request, and require the unseen prefix to be valid usage too.
    const lowered = previous !== null && CODEX_USAGE_FIELDS.some((key) => cumulative[key] < previous[key]);
    const rewound = lowered && latestCumulativeAtMs !== undefined && timestampMs > latestCumulativeAtMs;
    const reconciled =
      previous === null || rewound
        ? combineCodexUsage(cumulative, last, -1) !== null
        : CODEX_USAGE_FIELDS.every((key) => cumulative[key] - previous[key] === last[key]);
    if (!restarted && !reconciled) {
      const bridged = unreportedRequests(previous, cumulative, last, pending);
      if (bridged === null) {
        // A rejected snapshot from before the baseline's time is a replay, not the next baseline.
        const replayed =
          latestCumulativeAtMs !== undefined &&
          (timestampMs < latestCumulativeAtMs ||
            (timestampMs === latestCumulativeAtMs && lowered && cumulative.input_tokens + cumulative.output_tokens > 0));
        if (replayed) {
          state.lastCumulativeUsage = previous;
          if (pending !== undefined) state.unreportedRequests = pending;
        }
        state.malformedRecords += 1;
        return [];
      }
      unreported = bridged;
    } else if (!restarted && rewound) {
      state.malformedRecords += discardedRequests(pending, last);
    }
    if (restarted) state.sequenceStartSignature = sequenceStart;
  } else {
    if (signature === state.lastUsageSignature) return [];
    // Advance expected totals for billed legacy requests so a cumulative re-emission is stale.
    const advanced = combineCodexUsage(previous, last, 1);
    if (advanced === null) {
      state.malformedRecords += 1;
      return [];
    }
    state.lastCumulativeUsage = advanced;
    dateCodexBaseline(state, timestampMs);
  }
  state.lastUsageSignature = signature;

  return [
    ...unreported.map((request) => codexRequestRecord(state, request.usage, request)),
    codexRequestRecord(state, last, codexAttribution(state, timestampMs)),
  ];
}

// ---- Files ----

/** Where each provider keeps its transcripts, honoring the same variables the CLIs do. */
export function transcriptDirs(env: NodeJS.ProcessEnv = process.env, home = os.homedir()): Record<UsageProvider, string> {
  const claudeHome = env.CLAUDE_CONFIG_DIR?.trim() || path.join(home, ".claude");
  const codexHome = env.CODEX_HOME?.trim() || path.join(home, ".codex");
  return { claude: path.resolve(claudeHome, "projects"), codex: path.resolve(codexHome, "sessions") };
}

export interface TranscriptFile {
  path: string;
  size: number;
  mtimeMs: number;
}

/** Every .jsonl under dir written since sinceMs; null when dir doesn't exist. */
export async function listTranscripts(dir: string, sinceMs: number): Promise<TranscriptFile[] | null> {
  let entries;
  try {
    entries = await fs.readdir(dir, { recursive: true, withFileTypes: true });
  } catch (error) {
    if ((error as NodeJS.ErrnoException).code === "ENOENT") return null;
    throw error;
  }
  const files = await Promise.all(
    entries
      .filter((entry) => entry.isFile() && entry.name.endsWith(".jsonl"))
      .map(async (entry) => {
        const file = path.join(entry.parentPath, entry.name);
        try {
          const stats = await fs.stat(file);
          return stats.mtimeMs >= sinceMs ? { path: file, size: stats.size, mtimeMs: stats.mtimeMs } : null;
        } catch {
          return null;
        }
      }),
  );
  return files.filter((file) => file !== null);
}

// Lines without these can't carry usage or the state it depends on, so they're never parsed.
const MARKERS: Record<UsageProvider, string[]> = {
  claude: ['"usage"'],
  codex: ['"token_count"', '"token_usage_record"', '"turn_context"', '"thread_settings_applied"', '"session_meta"'],
};

/** A transcript's records, and how many usage lines in it were malformed. */
export async function readTranscript(provider: UsageProvider, file: string): Promise<{ records: UsageRecord[]; malformed: number }> {
  const text = await fs.readFile(file, "utf8");
  const state = initialCodexScanState();
  const records: UsageRecord[] = [];
  for (const line of text.split("\n")) {
    if (!MARKERS[provider].some((marker) => line.includes(marker))) continue;
    records.push(...(provider === "claude" ? parseClaudeLine(line) : parseCodexLine(line, state)));
  }
  return { records, malformed: state.malformedRecords };
}
