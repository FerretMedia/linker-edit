import { describe, expect, it } from "vitest";
import { mergeManifests } from "./release-assets.ts";

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
