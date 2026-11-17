import { describe, expect, it } from "vitest";
import { appendSegment, browseDirectory, browseFilter, filterEntries, parentDirectory, tabCompletion, trimSeparators } from "./browsePath.ts";

const entries = (...names: string[]) => names.map((name) => ({ name, isProject: false }));
/** A Windows path written with forward slashes, to spare the backslash escapes. */
const win = (value: string) => value.replaceAll("/", "\\");

describe("splitting what's typed", () => {
  it("splits into the folder to list and the filter after it", () => {
    expect([browseDirectory(win("C:/Users/jar")), browseFilter(win("C:/Users/jar"))]).toEqual([win("C:/Users/"), "jar"]);
    expect([browseDirectory(win("C:/Users/")), browseFilter(win("C:/Users/"))]).toEqual([win("C:/Users/"), ""]);
    expect([browseDirectory("/home/me/Vid"), browseFilter("/home/me/Vid")]).toEqual(["/home/me/", "Vid"]);
    expect([browseDirectory("~"), browseFilter("~")]).toEqual(["~/", ""]);
    expect(browseDirectory("Videos")).toBe("");
  });

  it("goes into a folder with the path's own separator", () => {
    expect(appendSegment(win("C:/Users/ja"), "jarno")).toBe(win("C:/Users/jarno/"));
    expect(appendSegment("/home/", "me")).toBe("/home/me/");
    expect(appendSegment("~/", "Videos")).toBe("~/Videos/");
  });

  it("goes up to a drive, a share or the root, and no further", () => {
    expect(parentDirectory(win("C:/Users/jarno/"))).toBe(win("C:/Users/"));
    expect(parentDirectory(win("C:/Users/"))).toBe(win("C:/"));
    expect(parentDirectory(win("C:/"))).toBeNull();
    expect(parentDirectory(win("//nas/media/Promo/"))).toBe(win("//nas/media/"));
    expect(parentDirectory(win("//nas/media/"))).toBeNull();
    expect(parentDirectory("/home/")).toBe("/");
    expect(parentDirectory("/")).toBeNull();
  });

  it("trims trailing separators but keeps a root's", () => {
    expect(trimSeparators(win("C:/Users/jarno/"))).toBe(win("C:/Users/jarno"));
    expect(trimSeparators(win("C:/"))).toBe(win("C:/"));
    expect(trimSeparators("C:")).toBe(win("C:/"));
    expect(trimSeparators("/")).toBe("/");
    expect(trimSeparators(" /media/ ")).toBe("/media");
  });
});

describe("filtering and Tab", () => {
  it("matches the start of names, ignoring case, and hides dot-folders until a dot is typed", () => {
    const list = entries(".git", "Promo", "promo cuts", "B-roll");
    expect(filterEntries(list, "pro").map((entry) => entry.name)).toEqual(["Promo", "promo cuts"]);
    expect(filterEntries(list, "").map((entry) => entry.name)).toEqual(["Promo", "promo cuts", "B-roll"]);
    expect(filterEntries(list, ".").map((entry) => entry.name)).toEqual([".git"]);
  });

  it("completes one match fully and several to what they share", () => {
    expect(tabCompletion(entries("Promo"), "pr")).toEqual({ name: "Promo", complete: true });
    expect(tabCompletion(entries("Promo final", "Promo draft"), "pr")).toEqual({ name: "Promo ", complete: false });
    expect(tabCompletion(entries("Promo", "Prores"), "pro")).toBeNull();
    expect(tabCompletion([], "x")).toBeNull();
  });
});
