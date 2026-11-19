import { describe, expect, it } from "vitest";
import { isReleaseAsset, mergeManifests } from "./release-assets.ts";

const manifest = (arch: string) => `version: 2026.1111.0
files:
  - url: Linker-Edit-2026.1111.0-mac-${arch}.zip
    sha512: zip-${arch}
    size: 1
  - url: Linker-Edit-2026.1111.0-mac-${arch}.dmg
    sha512: dmg-${arch}
    size: 2
path: Linker-Edit-2026.1111.0-mac-${arch}.zip
sha512: zip-${arch}
releaseDate: '2026-11-11T12:00:00.000Z'
`;

describe("mergeManifests", () => {
  it("lists both Mac builds' files under the first one's header", () => {
    expect(mergeManifests([manifest("arm64"), manifest("x64")])).toBe(`version: 2026.1111.0
files:
  - url: Linker-Edit-2026.1111.0-mac-arm64.zip
    sha512: zip-arm64
    size: 1
  - url: Linker-Edit-2026.1111.0-mac-arm64.dmg
    sha512: dmg-arm64
    size: 2
  - url: Linker-Edit-2026.1111.0-mac-x64.zip
    sha512: zip-x64
    size: 1
  - url: Linker-Edit-2026.1111.0-mac-x64.dmg
    sha512: dmg-x64
    size: 2
path: Linker-Edit-2026.1111.0-mac-arm64.zip
sha512: zip-arm64
releaseDate: '2026-11-11T12:00:00.000Z'
`);
  });

  it("leaves a single manifest as it is", () => {
    expect(mergeManifests([manifest("x64").replace(/\n/g, "\r\n")])).toBe(manifest("x64"));
  });
});

describe("isReleaseAsset", () => {
  it.each([
    ["Linker-Edit-2026.11.19-win-x64.exe", true],
    ["Linker-Edit-2026.11.19-mac-arm64.zip.blockmap", true],
    ["Linker-Edit-2026.11.19-web.tar.gz", true],
    ["Linker-Edit-2026.11.19-web.tar.gz.sha256", true],
    ["builder-debug.yml", false],
    ["Linker-Edit-2026.11.19-web.tar.gz.tmp", false],
  ])("%s → %s", (name, expected) => {
    expect(isReleaseAsset(name)).toBe(expected);
  });
});
