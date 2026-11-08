import { describe, expect, it } from "vitest";
import type { ChatItem } from "../shared/protocol.ts";
import { handoffSummary, withHandoff } from "./handoff.ts";

const chat: ChatItem[] = [
  { id: "1", kind: "user", text: "Cut the\n\nsilences" },
  { id: "2", kind: "tool", name: "mcp__resolve__timeline", input: { action: "list" }, status: "done" },
  { id: "3", kind: "tool", name: "mcp__resolve__timeline", input: { action: "create_empty", params: { name: "AI - Cut" } }, status: "done" },
  { id: "4", kind: "tool", name: "mcp__resolve__timeline", input: { action: "delete" }, status: "error" },
  { id: "5", kind: "notice", text: "Saved to edit history", tone: "info" },
  { id: "6", kind: "assistant", text: "a".repeat(300) },
];

describe("handoffSummary", () => {
  it("keeps what was said and done in Resolve, one short line each", () => {
    expect(handoffSummary(chat).split("\n")).toEqual([
      "User: Cut the silences",
      'Resolve action: timeline.create_empty {"name":"AI - Cut"}',
      `Assistant: ${"a".repeat(237)}...`,
    ]);
  });

  it("keeps the newest lines when the chat runs over the budget", () => {
    const summary = handoffSummary(chat, 300);
    expect(summary.split("\n")).toEqual(["(earlier messages left out)", `Assistant: ${"a".repeat(237)}...`]);
  });

  it("is empty before anyone said anything, and then adds nothing to the prompt", () => {
    expect(handoffSummary([{ id: "n", kind: "notice", text: "Stopped.", tone: "info" }])).toBe("");
    expect(withHandoff("", "hello")).toBe("hello");
  });
});
