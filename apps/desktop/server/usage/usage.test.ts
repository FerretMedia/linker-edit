import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { claudeUsageToLimits, codexRateLimitsToLimits } from "./limits.ts";
import { parseRateTable } from "./pricing.ts";
import { parseUsageWindow, summarize, usageReader } from "./summary.ts";
import { initialCodexScanState, parseClaudeLine, parseCodexLine } from "./transcripts.ts";

// Per million tokens: input $3, output $15, cache read $0.30, cache write $3.75; Codex's model $1 and $8.
const RATES = parseRateTable({
  "claude-sonnet-4-5": {
    input_cost_per_token: 3e-6,
    output_cost_per_token: 15e-6,
    cache_read_input_token_cost: 0.3e-6,
    cache_creation_input_token_cost: 3.75e-6,
  },
  "openai/gpt-5": { input_cost_per_token: 1e-6, output_cost_per_token: 8e-6, cache_read_input_token_cost: 0.1e-6 },
});

const DAY = { resolution: "day", sinceDay: "2026-11-20", untilDay: "2026-11-26", timeZone: "UTC" } as const;

const claudeLine = (fields: { id: string; at: string; output: number; model?: string; session?: string }) =>
  JSON.stringify({
    type: "assistant",
    timestamp: fields.at,
    requestId: `req_${fields.id}`,
    sessionId: fields.session ?? "s1",
    message: {
      id: fields.id,
      model: fields.model ?? "claude-sonnet-4-5",
      usage: { input_tokens: 1000, cache_read_input_tokens: 10_000, cache_creation_input_tokens: 2000, output_tokens: fields.output },
    },
  });

const codexLines = (lines: object[]) => {
  const state = initialCodexScanState();
  return lines.flatMap((line) => parseCodexLine(JSON.stringify(line), state));
};
const tokenCount = (at: string, last: object, total: object) => ({
  type: "event_msg",
  timestamp: at,
  payload: { type: "token_count", info: { last_token_usage: last, total_token_usage: total } },
});

describe("usage", () => {
  it("counts a Claude message once across its repeated blocks and prices it", () => {
    const records = [
      // Each content block repeats the message's usage, and a streamed copy can report less output.
      ...parseClaudeLine(claudeLine({ id: "m1", at: "2026-11-25T10:00:00Z", output: 100 })),
      ...parseClaudeLine(claudeLine({ id: "m1", at: "2026-11-25T10:00:01Z", output: 500 })),
      ...parseClaudeLine(claudeLine({ id: "m0", at: "2026-11-25T09:00:00Z", output: 7, model: "<synthetic>" })),
    ];
    const report = summarize(records, DAY, RATES);

    expect(report.totalTokens).toBe(1000 + 10_000 + 2000 + 500);
    expect(report.costUsd).toBeCloseTo(1000 * 3e-6 + 10_000 * 0.3e-6 + 2000 * 3.75e-6 + 500 * 15e-6);
    expect(report.cacheSavingsUsd).toBeCloseTo(10_000 * (3e-6 - 0.3e-6));
    expect(report.sessions).toBe(1);
    expect(report.periods.map((period) => period.period)).toEqual(["2026-11-25"]);
  });

  it("bills each Codex request once from its cumulative counters, cached input apart", () => {
    const first = { input_tokens: 1000, cached_input_tokens: 400, output_tokens: 50, reasoning_output_tokens: 20, total_tokens: 1050 };
    const second = { input_tokens: 2000, cached_input_tokens: 1500, output_tokens: 80, reasoning_output_tokens: 0, total_tokens: 2080 };
    const sum = { input_tokens: 3000, cached_input_tokens: 1900, output_tokens: 130, reasoning_output_tokens: 20, total_tokens: 3130 };
    const records = codexLines([
      { type: "session_meta", timestamp: "2026-11-24T08:00:00Z", payload: { id: "codex-session" } },
      { type: "turn_context", timestamp: "2026-11-24T08:00:00Z", payload: { model: "gpt-5" } },
      tokenCount("2026-11-24T08:00:05Z", first, first),
      // Codex re-emits a snapshot it already reported.
      tokenCount("2026-11-24T08:00:06Z", first, first),
      tokenCount("2026-11-24T08:00:09Z", second, sum),
    ]);
    const report = summarize(records, DAY, RATES);

    expect(report.uncachedInputTokens).toBe(600 + 500);
    expect(report.cachedInputTokens).toBe(400 + 1500);
    expect(report.outputTokens).toBe(130);
    expect(report.providers).toEqual([expect.objectContaining({ provider: "codex", sessions: 1, tokenShare: 1 })]);
    expect(report.models[0]).toMatchObject({ model: "gpt-5", unpricedRecords: 0 });
  });

  it("puts usage in the viewer's days or the last 24 hours, and leaves unknown models unpriced", () => {
    const records = [
      ...parseClaudeLine(claudeLine({ id: "a", at: "2026-11-25T23:30:00Z", output: 10 })),
      ...parseClaudeLine(claudeLine({ id: "b", at: "2026-11-26T09:00:00Z", output: 10, model: "claude-unknown-9" })),
      ...parseClaudeLine(claudeLine({ id: "c", at: "2026-11-10T09:00:00Z", output: 10 })),
    ];
    // 23:30 UTC is already the 26th in Amsterdam; the 10th is outside the week.
    const days = summarize(records, { ...DAY, timeZone: "Europe/Amsterdam" }, RATES);
    expect(days.periods.map((period) => period.period)).toEqual(["2026-11-26"]);
    expect(days.unpricedShare).toBe(0.5);
    expect(days.models.find((model) => model.model === "claude-unknown-9")).toMatchObject({ costUsd: 0, unpricedRecords: 1 });

    const hours = summarize(records, { resolution: "hour", sinceTime: "2026-11-25T09:30:00.000Z", untilTime: "2026-11-26T09:30:00.000Z", timeZone: "UTC" }, RATES);
    expect(hours.periods.map((period) => period.period)).toEqual(["2026-11-25T23:30:00.000Z", "2026-11-26T08:30:00.000Z"]);
  });

  it("reads transcripts from disk and picks up lines appended to one", async () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-usage-"));
    const claude = path.join(root, "projects", "C--work");
    fs.mkdirSync(claude, { recursive: true });
    const file = path.join(claude, "s1.jsonl");
    fs.writeFileSync(file, `${claudeLine({ id: "m1", at: new Date().toISOString(), output: 100 })}\n`);
    const today = new Intl.DateTimeFormat("en-CA", { timeZone: "UTC" }).format(new Date());
    const window = { resolution: "day", sinceDay: today, untilDay: today, timeZone: "UTC" } as const;
    const read = usageReader({ claude: path.join(root, "projects"), codex: path.join(root, "missing") }, async () => RATES);

    expect((await read(window)).outputTokens).toBe(100);
    fs.appendFileSync(file, `${claudeLine({ id: "m2", at: new Date().toISOString(), output: 50 })}\n`);
    const report = await read(window);
    expect(report.outputTokens).toBe(150);
    expect(report.notices).toEqual([]);
  });

  it("only takes a window of days, or of at most a day of hours", () => {
    expect(parseUsageWindow(new URLSearchParams("since=2026-11-20&until=2026-11-26&tz=Europe/Amsterdam"))).toEqual({
      resolution: "day",
      sinceDay: "2026-11-20",
      untilDay: "2026-11-26",
      timeZone: "Europe/Amsterdam",
    });
    expect(parseUsageWindow(new URLSearchParams("from=2026-11-25T10:00:00Z&to=2026-11-26T10:00:00Z"))).toMatchObject({ resolution: "hour" });
    expect(parseUsageWindow(new URLSearchParams("from=2026-11-20T10:00:00Z&to=2026-11-26T10:00:00Z"))).toBeNull();
    expect(parseUsageWindow(new URLSearchParams("since=2026-11-26&until=2026-11-20"))).toBeNull();
    expect(parseUsageWindow(new URLSearchParams("since=../etc&until=2026-11-20"))).toBeNull();
  });

  it("turns each provider's limits into windows, or says why there are none", () => {
    expect(
      claudeUsageToLimits({
        rate_limits_available: true,
        rate_limits: {
          five_hour: { utilization: 42, resets_at: "2026-11-26T15:00:00Z" },
          seven_day: { utilization: 130, resets_at: null },
          model_scoped: [{ display_name: "Opus", utilization: 10, resets_at: null }],
        },
      }).windows,
    ).toEqual([
      { id: "five_hour", kind: "session", label: "Session", windowDurationMins: 300, usedPercent: 42, resetsAt: "2026-11-26T15:00:00.000Z" },
      { id: "seven_day", kind: "weekly", label: "Weekly", windowDurationMins: 10080, usedPercent: 100 },
      { id: "seven_day_opus", kind: "weekly", label: "Weekly · Opus", windowDurationMins: 10080, usedPercent: 10 },
    ]);
    expect(claudeUsageToLimits({ rate_limits_available: false, rate_limits: null })).toMatchObject({ windows: [], unavailable: expect.any(String) });

    // Free and Go plans' first window is monthly.
    expect(codexRateLimitsToLimits({ planType: "free", primary: { usedPercent: 30, resetsAt: 1_800_000_000 } }).windows).toEqual([
      { id: "primary", kind: "monthly", label: "Monthly", usedPercent: 30, windowDurationMins: 43200, resetsAt: "2027-01-15T08:00:00.000Z" },
    ]);
    expect(codexRateLimitsToLimits({ limitId: "gpt-5-pro", primary: { usedPercent: 30 } })).toMatchObject({ windows: [], unavailable: expect.any(String) });
  });
});
