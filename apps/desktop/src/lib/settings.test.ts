import { describe, expect, it } from "vitest";
import { resolveStatusLine, sectionFromHash, settingsHash } from "./settings.ts";

describe("settings address", () => {
  it.each([
    ["", null],
    ["#", null],
    ["#settings", "general"],
    ["#settings/remote", "remote"],
    ["#settings/nonsense", "general"],
    ["#settingsx", null],
  ])("reads %j as %j", (hash, section) => {
    expect(sectionFromHash(hash)).toBe(section);
  });

  it("goes back and forth", () => {
    expect(sectionFromHash(settingsHash("transcription"))).toBe("transcription");
  });
});

describe("resolveStatusLine", () => {
  it("names the version without its build number", () => {
    const line = resolveStatusLine({
      state: "connected",
      product: "DaVinci Resolve Studio",
      version: "21.1.2.5",
      project: "Promo",
      projectId: null,
      database: null,
      timeline: "Cut 1",
    });
    expect(line).toEqual({ tone: "bg-success", title: "DaVinci Resolve Studio 21.1.2", detail: "Promo · Cut 1" });
  });
});
