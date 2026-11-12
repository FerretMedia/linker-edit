import { describe, expect, it } from "vitest";
import { releaseName } from "#shared/release-name.ts";
import { isNewer } from "../server/updates.ts";
import { releaseVersion } from "./release-version.ts";

describe("releaseVersion", () => {
  it("is the UTC day and time", () => {
    expect(releaseVersion(new Date("2026-11-08T14:30:59Z"))).toBe("2026.1108.1430");
    expect(releaseVersion(new Date("2026-11-08T00:05:00Z"))).toBe("2026.1108.5");
    expect(releaseVersion(new Date("2027-01-05T00:00:00Z"))).toBe("2027.105.0");
  });

  it("sorts a day's later release after its earlier one", () => {
    expect(isNewer(releaseVersion(new Date("2026-11-08T14:30:00Z")), releaseVersion(new Date("2026-11-08T09:59:00Z")))).toBe(true);
  });

  it("sorts after the previous day's releases", () => {
    expect(isNewer(releaseVersion(new Date("2026-02-01T00:00:00Z")), "2026.131.2359")).toBe(true);
    expect(isNewer(releaseVersion(new Date("2026-10-01T00:00:00Z")), "2026.930.2359")).toBe(true);
  });
});

describe("releaseName", () => {
  it("is just the day", () => {
    expect(releaseName("2026.1107.0")).toBe("2026.11.07");
    expect(releaseName("2026.1111.1430")).toBe("2026.11.11");
    expect(releaseName("2027.105.2359")).toBe("2027.01.05");
  });

  it("leaves anything that isn't a release version alone", () => {
    expect(releaseName("0.0.0")).toBe("0.0.0");
    expect(releaseName("1.2.3")).toBe("1.2.3");
  });
});
