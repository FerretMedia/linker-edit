import { describe, expect, it } from "vitest";
import { isNewer } from "../server/updates.ts";
import { nextVersion } from "./release-version.ts";

describe("nextVersion", () => {
  const nov8 = new Date("2026-11-08T23:30:00Z");

  it("starts each UTC day at .0 and counts up from that day's highest tag", () => {
    expect(nextVersion(nov8, [])).toBe("2026.1108.0");
    expect(nextVersion(nov8, ["v2026.1108.0", "v2026.1108.2", "v2026.1107.5", "v2026.11080.9"])).toBe("2026.1108.3");
    expect(nextVersion(new Date("2027-01-05T00:00:00Z"), ["v2026.1231.4"])).toBe("2027.105.0");
  });

  it("sorts after the previous day's releases", () => {
    expect(isNewer(nextVersion(new Date("2026-02-01T00:00:00Z"), []), "2026.131.7")).toBe(true);
    expect(isNewer(nextVersion(new Date("2026-10-01T00:00:00Z"), []), "2026.930.0")).toBe(true);
  });
});
