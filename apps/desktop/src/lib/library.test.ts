import { describe, expect, it } from "vitest";
import { describeCounts, fileUrl, formatDuration, libraryHash, libraryTabFromHash, parseTags, samePath } from "./library.ts";

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

describe("sorting an assets folder", () => {
  it("says what a folder holds, in a few words", () => {
    expect(describeCounts({ audio: 124, video: 1, image: 0, lut: 0, other: 2 })).toBe("124 audio · 1 video · 2 other");
    expect(describeCounts({ audio: 0, video: 0, image: 3, lut: 1, other: 0 }, true)).toBe("3+ images · 1+ LUT");
    expect(describeCounts({ audio: 0, video: 0, image: 0, lut: 0, other: 0 })).toBe("No files");
  });

  it("knows a folder already in the library, whatever the case on Windows", () => {
    expect(samePath("D:\\Video-Editing\\Music", "d:\\video-editing\\music\\")).toBe(true);
    expect(samePath("/home/me/Music", "/home/me/music")).toBe(false);
    expect(samePath("/home/me/Music/", "/home/me/Music")).toBe(true);
  });
});
