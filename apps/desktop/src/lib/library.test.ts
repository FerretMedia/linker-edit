import { describe, expect, it } from "vitest";
import { fileUrl, formatDuration, libraryHash, libraryTabFromHash, parseTags } from "./library.ts";

describe("library page helpers", () => {
  it("reads the tab from the address and back", () => {
    expect(libraryTabFromHash("#library")).toBe("universal");
    expect(libraryTabFromHash("#library/project")).toBe("project");
    expect(libraryTabFromHash("#library/nonsense")).toBe("universal");
    expect(libraryTabFromHash("#settings/general")).toBeNull();
    expect(libraryHash("project")).toBe("#library/project");
    expect(libraryHash("universal")).toBe("#library");
  });

  it("writes lengths the way editors read them", () => {
    expect(formatDuration(1.23)).toBe("1.2s");
    expect(formatDuration(187.4)).toBe("3:07");
    expect(formatDuration(3723)).toBe("1:02:03");
  });

  it("encodes each part of a file's path", () => {
    expect(fileUrl("id", "Calm/Piano #1.wav")).toBe("/library/id/Calm/Piano%20%231.wav");
  });

  it("splits tags on commas", () => {
    expect(parseTags(" happy, intro ,, ")).toEqual(["happy", "intro"]);
  });
});
