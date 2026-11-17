import { describe, expect, it } from "vitest";
import { isNewer } from "./updates.ts";

describe("isNewer", () => {
  it("compares numerically, not as text", () => {
    expect(isNewer("2026.1108.0", "2026.205.3")).toBe(true);
    expect(isNewer("2026.205.10", "2026.205.9")).toBe(true);
    expect(isNewer("2026.205.0", "2026.205.0")).toBe(false);
    expect(isNewer("0.0.0", "2026.205.0")).toBe(false);
  });
});
