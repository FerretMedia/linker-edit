import { describe, expect, it } from "vitest";
import { releaseName, versionOfName } from "#shared/release-name.ts";
import { isNewer } from "../server/updates.ts";
import { nextVersion } from "./release-version.ts";

describe("nextVersion", () => {
  const nov8 = new Date("2026-11-08T23:30:00Z");

  it("starts each UTC day at .0 and counts up from that day's highest tag", () => {
    expect(nextVersion(nov8, [])).toBe("2026.1108.0");
    expect(nextVersion(nov8, ["v2026.11.08", "v2026.11.08.3", "v2026.11.07.6", "v2026.11.080"])).toBe("2026.1108.3");
    expect(nextVersion(new Date("2027-01-05T00:00:00Z"), ["v2026.12.31.5"])).toBe("2027.105.0");
  });

  it("counts the releases tagged before the names", () => {
    expect(nextVersion(nov8, ["v2026.1108.0", "v2026.1108.1"])).toBe("2026.1108.2");
    expect(nextVersion(nov8, ["v2026.1108.0", "v2026.11.08.2"])).toBe("2026.1108.2");
  });

  it("sorts after the previous day's releases", () => {
    expect(isNewer(nextVersion(new Date("2026-02-01T00:00:00Z"), []), "2026.131.7")).toBe(true);
    expect(isNewer(nextVersion(new Date("2026-10-01T00:00:00Z"), []), "2026.930.0")).toBe(true);
  });
});

describe("releaseName", () => {
  it("names a release by its day, and counts further releases that day from .2", () => {
    expect(releaseName("2026.1107.0")).toBe("2026.11.07");
    expect(releaseName("2026.1111.1")).toBe("2026.11.11.2");
    expect(releaseName("2027.105.0")).toBe("2027.01.05");
  });

  it("leaves anything that isn't a release version alone", () => {
    expect(releaseName("0.0.0")).toBe("0.0.0");
    expect(releaseName("1.2.3")).toBe("1.2.3");
  });

  it("maps back to the version", () => {
    for (const version of ["2026.1107.0", "2026.1111.1", "2027.105.0", "2026.1231.9"]) {
      expect(versionOfName(releaseName(version))).toBe(version);
    }
    expect(versionOfName("2026.11.07.1")).toBeNull();
    expect(versionOfName("2026.1107.0")).toBeNull();
  });
});
