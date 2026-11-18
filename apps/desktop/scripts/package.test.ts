import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
import { bridgeManifests, channel } from "./package.ts";
import { stampCargoLock, stampCargoToml } from "./stamp-version.ts";

describe("stamping the version", () => {
  it("sets the workspace's version and its crates' in the lockfile, nothing else", () => {
    const toml = `[workspace]\nmembers = ["crates/*"]\n\n[workspace.package]\nversion = "0.0.0"\nedition = "2024"\n\n[workspace.dependencies]\nserde = { version = "1" }\n`;
    expect(stampCargoToml(toml, "2026.1118.1430")).toBe(toml.replace('version = "0.0.0"', 'version = "2026.1118.1430"'));
    const lock = `[[package]]\nname = "linker-edit"\nversion = "0.0.0"\n\n[[package]]\nname = "serde"\nversion = "1.0.0"\n`;
    expect(stampCargoLock(lock, ["linker-edit"], "2026.1118.1430")).toBe(lock.replace('"0.0.0"', '"2026.1118.1430"'));
  });
});

describe("bridgeManifests", () => {
  const data = (file: string) => Buffer.from(`bytes of ${file}`);
  const sha512 = (file: string) => createHash("sha512").update(data(file)).digest("base64");
  const files = [
    "Linker-Edit-win-x64-Setup.exe",
    "Linker-Edit-osx-x64-Portable.zip",
    "Linker-Edit-osx-arm64-Portable.zip",
    "Linker-Edit-osx-arm64-Setup.pkg",
    "Linker-Edit-linux-x64.AppImage",
    "releases.linux-x64.json",
  ];

  it("offers the Mac zips and the Linux AppImage to the Electron apps, and Windows nothing", () => {
    const manifests = bridgeManifests(files, data, "2026.1118.1430", "2026-11-18T14:30:00.000Z");
    expect([...manifests.keys()]).toEqual(["latest-mac.yml", "latest-linux.yml"]);
    expect(manifests.get("latest-linux.yml")).toBe(`version: 2026.1118.1430
files:
  - url: Linker-Edit-linux-x64.AppImage
    sha512: ${sha512("Linker-Edit-linux-x64.AppImage")}
    size: ${data("Linker-Edit-linux-x64.AppImage").length}
path: Linker-Edit-linux-x64.AppImage
sha512: ${sha512("Linker-Edit-linux-x64.AppImage")}
releaseDate: '2026-11-18T14:30:00.000Z'
`);
    expect(manifests.get("latest-mac.yml")).toContain("  - url: Linker-Edit-osx-arm64-Portable.zip\n");
    expect(manifests.get("latest-mac.yml")).toContain("  - url: Linker-Edit-osx-x64-Portable.zip\n");
  });

  it("names Velopack's channels after the platform", () => {
    expect(channel("win32-x64")).toBe("win-x64");
    expect(channel("darwin-arm64")).toBe("osx-arm64");
    expect(channel("linux-x64")).toBe("linux-x64");
  });
});
