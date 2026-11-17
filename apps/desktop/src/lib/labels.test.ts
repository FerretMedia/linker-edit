import { describe, expect, it } from "vitest";
import { briefRequest, isResolveTool, lineDiff, toolLabel } from "./labels.ts";

describe("a change to the brief", () => {
  it("shows new rules against the old, line by line and in order", () => {
    const diff = lineDiff("Captions in white.\nEnd on the logo.\nNo swearing.\n", "Captions in white.\nIntro at most 3 seconds.\nNo swearing.");
    expect(diff.map((line) => `${line.kind}: ${line.text}`)).toEqual([
      "same: Captions in white.",
      "removed: End on the logo.",
      "added: Intro at most 3 seconds.",
      "same: No swearing.",
    ]);
    // A first brief is all new.
    expect(lineDiff("", "One.\nTwo.")).toEqual([
      { kind: "added", text: "One." },
      { kind: "added", text: "Two." },
    ]);
  });

  it("reads what save_brief and add_lesson ask to change, and nothing else as a brief", () => {
    expect(briefRequest("mcp__linker__add_lesson", { for: 'Series "Reels"', feedback: "the start drags", rule: "Intro at most 3 seconds." })).toEqual({
      kind: "lesson",
      for: 'Series "Reels"',
      feedback: "the start drags",
      rule: "Intro at most 3 seconds.",
    });
    expect(briefRequest("mcp__linker__save_brief", { for: "W", rules: "x" })).toMatchObject({ kind: "rules", was: "", rules: "x" });
    expect(briefRequest("mcp__linker__add_clips", {})).toBeNull();
  });

  it("doesn't call the brief and study tools Resolve tools, and still shows their short names", () => {
    expect(isResolveTool("mcp__linker__add_clips")).toBe(true);
    expect(["save_brief", "add_lesson", "save_example", "study_media", "compare_cut", "transcribe"].some((tool) => isResolveTool(`mcp__linker__${tool}`))).toBe(false);
    expect(toolLabel("mcp__linker__study_media", {})).toBe("study_media");
    expect(toolLabel("Bash", {})).toBe("Bash");
  });
});
