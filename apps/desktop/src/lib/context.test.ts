import { describe, expect, it } from "vitest";
import type { ContextUsage } from "../../shared/protocol.ts";
import { compactSuggested, formatTokens } from "./context.ts";

const expiresAt = "2026-11-08T12:00:00Z";
const before = Date.parse(expiresAt) - 1;
const after = Date.parse(expiresAt);
const big: ContextUsage = { usedTokens: 250_000, maxTokens: 1_000_000, cacheExpiresAt: expiresAt };

describe("compactSuggested", () => {
  it.each([
    ["a big Claude chat whose cache expired", "claude", big, after, true],
    ["while the cache is still warm", "claude", big, before, false],
    ["a small chat", "claude", { ...big, usedTokens: 150_000 }, after, false],
    ["an unknown cache lifetime", "claude", { ...big, cacheExpiresAt: null }, after, false],
    ["Codex", "codex", big, after, false],
  ] as const)("%s → %s", (_name, provider, context, now, expected) => {
    expect(compactSuggested(provider, context, now)).toBe(expected);
  });
});

describe("formatTokens", () => {
  it.each([
    [950, "950"],
    [84_400, "84K"],
    [1_000_000, "1M"],
    [1_250_000, "1.3M"],
  ])("%d → %s", (tokens, label) => {
    expect(formatTokens(tokens)).toBe(label);
  });
});
