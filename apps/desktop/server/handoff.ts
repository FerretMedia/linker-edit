import type { ChatItem } from "../shared/protocol.ts";
import { resolveWriteAction } from "./history/history.ts";

// As in T3 Code's portable handoffs: a short text summary, not the other agent's real context.
const ITEM_CHARS = 240;
const BUDGET_CHARS = 12_000;

function oneLine(text: string): string {
  const line = text.replace(/\s+/g, " ").trim();
  return line.length > ITEM_CHARS ? `${line.slice(0, ITEM_CHARS - 3)}...` : line;
}

/** The chat so far as short lines, newest kept when it runs over the budget; empty when nothing was said. */
export function handoffSummary(items: ChatItem[], budget = BUDGET_CHARS): string {
  const lines = items.flatMap((item) => {
    if (item.kind === "user") return [`User: ${oneLine(item.text)}`];
    if (item.kind === "assistant") return [`Assistant: ${oneLine(item.text)}`];
    if (item.kind === "tool" && item.status === "done") {
      const action = resolveWriteAction(item.name, item.input);
      return action ? [`Resolve action: ${action}`] : [];
    }
    return [];
  });
  if (!lines.some((line) => line.startsWith("User: "))) return "";
  const kept: string[] = [];
  let size = 0;
  for (const line of lines.toReversed()) {
    if (size + line.length + 1 > budget) {
      kept.push("(earlier messages left out)");
      break;
    }
    kept.push(line);
    size += line.length + 1;
  }
  return kept.reverse().join("\n");
}

/** Puts the summary ahead of the first message to a new agent session in an ongoing chat. */
export function withHandoff(summary: string, text: string): string {
  if (!summary) return text;
  return `<earlier_conversation>
This chat started in another agent session whose context you don't have. A summary, oldest first, with long messages cut:
${summary}
</earlier_conversation>

${text}`;
}
