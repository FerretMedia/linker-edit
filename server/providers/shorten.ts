// Resolve tools can return thousands of timeline items or transcript lines in one call. All of it
// would stay in the agent's context for the rest of the chat, so long results are cut first.

/** About 5,000 tokens. */
export const MAX_TOOL_OUTPUT_CHARS = 20_000;

const ITEM_LIMITS = [50, 20, 10, 5, 2];

/** A JSON result with its long lists capped, or a plain cut when it isn't JSON; null when it fits. */
export function shortenText(text: string, max = MAX_TOOL_OUTPUT_CHARS): string | null {
  if (text.length <= max) return null;
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    return cut(text, max);
  }
  for (const limit of ITEM_LIMITS) {
    // Compact JSON: indentation would cost tokens too.
    const shortened = JSON.stringify(capLists(parsed, limit));
    if (shortened.length <= max) return `${shortened}\n${NOTE}`;
  }
  return cut(JSON.stringify(capLists(parsed, ITEM_LIMITS.at(-1)!)), max);
}

const NOTE =
  "Linker-Edit shortened this result to save context. Ask again for a narrower part (one track, a range, a filter) to see what was left out.";

function cut(text: string, max: number): string {
  return `${text.slice(0, max)}\n… ${text.length - max} more characters left out. ${NOTE}`;
}

function capLists(value: unknown, limit: number): unknown {
  if (Array.isArray(value)) {
    const kept = value.slice(0, limit).map((item) => capLists(item, limit));
    return value.length > limit ? [...kept, `… ${value.length - limit} more items left out`] : kept;
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value).map(([key, item]) => [key, capLists(item, limit)]));
  }
  return value;
}

type ContentBlock = { type: string; text?: string };

/**
 * A PostToolUse hook's replacement for an MCP result, or undefined to leave it alone. Claude passes
 * MCP results as an array of content blocks; the object form is MCP's own CallToolResult.
 */
export function shortenToolOutput(output: unknown, max = MAX_TOOL_OUTPUT_CHARS): unknown {
  const blocks = Array.isArray(output)
    ? (output as ContentBlock[])
    : output && typeof output === "object" && Array.isArray((output as { content?: unknown }).content)
      ? ((output as { content: ContentBlock[] }).content)
      : null;
  if (!blocks) return undefined;
  let changed = false;
  const shortened = blocks.map((block) => {
    const text = block.type === "text" && typeof block.text === "string" ? shortenText(block.text, max) : null;
    if (text === null) return block;
    changed = true;
    return { ...block, text };
  });
  if (!changed) return undefined;
  if (Array.isArray(output)) return shortened;
  // structuredContent repeats the full result; the shortened text stands in for it.
  const { structuredContent: _full, ...rest } = output as Record<string, unknown>;
  return { ...rest, content: shortened };
}
