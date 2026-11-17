import { COMPACT_SUGGESTION_MIN_TOKENS, type ContextUsage, type Provider } from "#shared/protocol.ts";

/** 84000 → "84K", 1200000 → "1.2M". */
export function formatTokens(tokens: number): string {
  if (tokens >= 1_000_000) return `${Number((tokens / 1_000_000).toFixed(1))}M`;
  if (tokens >= 1_000) return `${Math.round(tokens / 1_000)}K`;
  return String(tokens);
}

/**
 * Whether to offer compacting before the next message: the chat is big and Claude's prompt cache
 * has expired, so the next turn would reread all of it uncached anyway.
 */
export function compactSuggested(provider: Provider, context: ContextUsage | null, nowMs: number): boolean {
  if (provider !== "claude" || !context?.cacheExpiresAt) return false;
  return context.usedTokens >= COMPACT_SUGGESTION_MIN_TOKENS && nowMs >= Date.parse(context.cacheExpiresAt);
}
