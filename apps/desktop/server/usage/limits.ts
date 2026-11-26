// Subscription limits, read as T3 Code reads them: Claude's through the Agent SDK's usage request, Codex's through
// `codex app-server` (apps/server/src/provider/claudeUsageLimits.ts, codexUsageLimits.ts, ClaudeProvider.ts and
// CodexProvider.ts; MIT, see NOTICE). Neither sends a prompt, so reading them costs no tokens.
import { spawn } from "node:child_process";
import { createRequire } from "node:module";
import * as os from "node:os";
import * as readline from "node:readline";
import { fileURLToPath } from "node:url";
import type { UsageLimits, UsageLimitWindow } from "../../shared/usage.ts";

const SESSION_MINS = 5 * 60;
const WEEK_MINS = 7 * 24 * 60;
const MONTH_MINS = 30 * 24 * 60;
const TIMEOUT_MS = 20_000;

const clampPercent = (value: number) => Math.max(0, Math.min(100, value));

function isoFromString(value: string | null | undefined): string | undefined {
  if (!value) return undefined;
  const ms = Date.parse(value);
  return Number.isNaN(ms) ? undefined : new Date(ms).toISOString();
}

function isoFromEpochSeconds(value: number | null | undefined): string | undefined {
  return typeof value === "number" && Number.isFinite(value) && value > 0 ? new Date(value * 1000).toISOString() : undefined;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

// ---- Claude ----

/** The slice of the Agent SDK's usage answer this reads; get_usage percentages are already 0 to 100. */
export interface ClaudeUsageResponse {
  rate_limits_available: boolean;
  rate_limits: {
    five_hour?: { utilization: number | null; resets_at: string | null } | null;
    seven_day?: { utilization: number | null; resets_at: string | null } | null;
    model_scoped?: unknown;
    extra_usage?: { is_enabled?: boolean; monthly_limit?: number | null; used_credits?: number | null; utilization?: number | null } | null;
  } | null;
}

const CLAUDE_WINDOWS = {
  five_hour: { kind: "session", label: "Session", windowDurationMins: SESSION_MINS },
  seven_day: { kind: "weekly", label: "Weekly", windowDurationMins: WEEK_MINS },
} as const;

function percentFromAmounts(used: number | null | undefined, limit: number | null | undefined) {
  if (typeof used !== "number" || typeof limit !== "number" || !Number.isFinite(used) || !Number.isFinite(limit) || used < 0 || limit <= 0) {
    return undefined;
  }
  return clampPercent((used / limit) * 100);
}

export function claudeUsageToLimits(response: ClaudeUsageResponse): UsageLimits {
  if (!response.rate_limits_available || !response.rate_limits) {
    return {
      provider: "claude",
      windows: [],
      unavailable: response.rate_limits_available ? "Could not read limits." : "This account has no subscription limits.",
    };
  }
  const limits = response.rate_limits;
  const windows: UsageLimitWindow[] = [];
  for (const id of ["five_hour", "seven_day"] as const) {
    const window = limits[id];
    if (!window || typeof window.utilization !== "number") continue;
    const resetsAt = isoFromString(window.resets_at);
    windows.push({ id, ...CLAUDE_WINDOWS[id], usedPercent: clampPercent(window.utilization), ...(resetsAt ? { resetsAt } : {}) });
  }
  // model_scoped shipped in the CLI after the SDK typings, so it is read structurally.
  for (const entry of Array.isArray(limits.model_scoped) ? (limits.model_scoped as unknown[]) : []) {
    const scoped = entry as { display_name?: unknown; utilization?: unknown; resets_at?: unknown };
    if (typeof scoped.display_name !== "string" || typeof scoped.utilization !== "number") continue;
    const resetsAt = isoFromString(typeof scoped.resets_at === "string" ? scoped.resets_at : null);
    windows.push({
      id: `seven_day_${scoped.display_name.toLowerCase().replace(/[^a-z0-9]+/g, "_")}`,
      kind: "weekly",
      label: `Weekly · ${scoped.display_name}`,
      windowDurationMins: WEEK_MINS,
      usedPercent: clampPercent(scoped.utilization),
      ...(resetsAt ? { resetsAt } : {}),
    });
  }
  const extra = limits.extra_usage;
  if (extra?.is_enabled === true) {
    const percent = percentFromAmounts(extra.used_credits, extra.monthly_limit) ?? extra.utilization;
    if (typeof percent === "number" && Number.isFinite(percent) && percent >= 0) {
      windows.push({ id: "monthly_spend", kind: "monthly", label: "Monthly spend", usedPercent: clampPercent(percent) });
    }
  }
  return { provider: "claude", windows, unavailable: windows.length === 0 ? "No limits reported." : null };
}

/** Starts Claude Code with a prompt that never comes, asks for its usage, and stops it. */
async function readClaude(cwd: string): Promise<UsageLimits> {
  const { query } = await import("@anthropic-ai/claude-agent-sdk");
  const abortController = new AbortController();
  const timer = setTimeout(() => abortController.abort(), TIMEOUT_MS);
  try {
    const q = query({
      prompt: (async function* () {
        await new Promise((resolve) => abortController.signal.addEventListener("abort", resolve, { once: true }));
      })(),
      options: {
        cwd,
        abortController,
        persistSession: false,
        settings: { disableAllHooks: true },
        allowedTools: [],
        mcpServers: {},
        strictMcpConfig: true,
        env: { ...process.env, ENABLE_CLAUDEAI_MCP_SERVERS: "false", CLAUDE_CODE_AUTO_CONNECT_IDE: "0", CLAUDE_CODE_IDE_SKIP_AUTO_INSTALL: "1" },
        stderr: () => {},
      },
    });
    await q.initializationResult();
    // Only the rate limits are read, so the scan of local transcripts that fills `behaviors` is skipped.
    const usage = await q.usage_EXPERIMENTAL_MAY_CHANGE_DO_NOT_RELY_ON_THIS_API_YET({ skipBehaviors: true });
    return claudeUsageToLimits(usage);
  } finally {
    clearTimeout(timer);
    abortController.abort();
  }
}

// ---- Codex ----

interface CodexRateLimitWindow {
  usedPercent: number;
  resetsAt?: number | null;
  windowDurationMins?: number | null;
}

export interface CodexRateLimitSnapshot {
  limitId?: string | null;
  planType?: string | null;
  primary?: CodexRateLimitWindow | null;
  secondary?: CodexRateLimitWindow | null;
  individualLimit?: { remainingPercent: number; resetsAt: number } | null;
}

function kindForDuration(mins: number): UsageLimitWindow["kind"] {
  if (mins >= MONTH_MINS) return "monthly";
  if (mins >= WEEK_MINS) return "weekly";
  return "session";
}

/** primary/secondary are positions; without windowDurationMins, paid plans use 5-hour/weekly windows and Free/Go monthly. */
export function codexRateLimitsToLimits(snapshot: CodexRateLimitSnapshot | null | undefined): UsageLimits {
  const windows: UsageLimitWindow[] = [];
  // Model-specific limits stay out of the main rows; older CLIs omit limit IDs.
  if (snapshot && (!snapshot.limitId || snapshot.limitId === "codex")) {
    const isMonthlyPlan = snapshot.planType === "free" || snapshot.planType === "go";
    const positions = [
      ["primary", snapshot.primary, isMonthlyPlan ? MONTH_MINS : SESSION_MINS],
      ["secondary", snapshot.secondary, WEEK_MINS],
    ] as const;
    for (const [id, window, fallbackMins] of positions) {
      if (!window || !Number.isFinite(window.usedPercent)) continue;
      const windowDurationMins = typeof window.windowDurationMins === "number" ? window.windowDurationMins : fallbackMins;
      const kind = kindForDuration(windowDurationMins);
      const resetsAt = isoFromEpochSeconds(window.resetsAt);
      windows.push({
        id,
        kind,
        label: kind === "session" ? "Session" : kind === "weekly" ? "Weekly" : "Monthly",
        usedPercent: clampPercent(window.usedPercent),
        windowDurationMins,
        ...(resetsAt ? { resetsAt } : {}),
      });
    }
    const individual = snapshot.individualLimit;
    if (individual && Number.isFinite(individual.remainingPercent)) {
      const resetsAt = isoFromEpochSeconds(individual.resetsAt);
      windows.push({
        id: "individual",
        kind: "monthly",
        label: "Monthly credit limit",
        usedPercent: clampPercent(100 - individual.remainingPercent),
        ...(resetsAt ? { resetsAt } : {}),
      });
    }
  }
  return { provider: "codex", windows, unavailable: windows.length === 0 ? "No limits reported." : null };
}

/** The Codex CLI that @openai/codex-sdk runs, started through its Node launcher so every platform finds its binary. */
function codexLauncher(): string {
  return createRequire(fileURLToPath(import.meta.resolve("@openai/codex-sdk"))).resolve("@openai/codex/bin/codex.js");
}

/** Asks `codex app-server` over its line-delimited JSON-RPC for the signed-in account's rate limits. */
async function readCodex(version: string): Promise<UsageLimits> {
  const child = spawn(process.execPath, [codexLauncher(), "app-server"], { stdio: ["pipe", "pipe", "ignore"], windowsHide: true });
  const pending = new Map<number, { resolve: (value: unknown) => void; reject: (error: Error) => void }>();
  let nextId = 0;
  const failAll = (error: Error) => {
    for (const { reject } of pending.values()) reject(error);
    pending.clear();
  };
  child.on("error", failAll);
  child.on("exit", () => failAll(new Error("Codex stopped before it answered.")));
  readline.createInterface({ input: child.stdout }).on("line", (line) => {
    let message: { id?: unknown; result?: unknown; error?: { message?: string } };
    try {
      message = JSON.parse(line) as typeof message;
    } catch {
      return;
    }
    const waiting = typeof message.id === "number" ? pending.get(message.id) : undefined;
    if (!waiting) return;
    pending.delete(message.id as number);
    if (message.error) waiting.reject(new Error(message.error.message ?? "Codex refused the request."));
    else waiting.resolve(message.result);
  });
  const send = (message: object) => child.stdin.write(`${JSON.stringify(message)}\n`);
  const request = (method: string, params: unknown) =>
    new Promise<unknown>((resolve, reject) => {
      const id = ++nextId;
      pending.set(id, { resolve, reject });
      send({ id, method, params });
    });
  const timer = setTimeout(() => failAll(new Error("Codex did not answer the usage request.")), TIMEOUT_MS);
  try {
    await request("initialize", { clientInfo: { name: "linker-edit", title: "Linker-Edit", version }, capabilities: { experimentalApi: true } });
    send({ method: "initialized" });
    const account = (await request("account/read", {})) as { account?: { type?: string } | null } | null;
    // Only a ChatGPT sign-in has a plan's quota; an API key pays per token.
    if (account?.account?.type !== "chatgpt") return { provider: "codex", windows: [], unavailable: "This account has no subscription limits." };
    const response = (await request("account/rateLimits/read", null)) as {
      rateLimits?: CodexRateLimitSnapshot;
      rateLimitsByLimitId?: Record<string, CodexRateLimitSnapshot> | null;
    } | null;
    return codexRateLimitsToLimits(response?.rateLimitsByLimitId?.codex ?? response?.rateLimits);
  } finally {
    clearTimeout(timer);
    child.kill();
  }
}

/** Both providers' limits; a provider that can't be read says why instead of failing the other. */
export async function readLimits(version: string, cwd = os.tmpdir()): Promise<UsageLimits[]> {
  return Promise.all([
    readCodex(version).catch((error: unknown) => ({ provider: "codex" as const, windows: [], unavailable: `Could not read limits: ${errorText(error)}` })),
    readClaude(cwd).catch((error: unknown) => ({ provider: "claude" as const, windows: [], unavailable: `Could not read limits: ${errorText(error)}` })),
  ]);
}
