import { describe, expect, it } from "vitest";
import { tarBuild } from "./tools.ts";

describe("tarBuild", () => {
  it("downloads Linker-Edit's own Mac builds: Metal on Apple silicon, the CPU on Intel", () => {
    expect(tarBuild("darwin", "arm64")).toMatchObject({ cli: "whisper-cli-macos-arm64/whisper-cli", device: "Metal" });
    expect(tarBuild("darwin", "x64")).toMatchObject({ cli: "whisper-cli-macos-x64/whisper-cli", device: "CPU" });
    expect(tarBuild("darwin", "arm64")?.url).toMatch(/^https:\/\/github\.com\/FerretMedia\/linker-edit\/releases\/download\//);
  });

  it("keeps Linux on whisper.cpp's own CPU builds", () => {
    expect(tarBuild("linux", "x64")?.cli).toBe("whisper-bin-ubuntu-x64/whisper-cli");
    expect(tarBuild("linux", "arm64")?.cli).toBe("whisper-bin-ubuntu-arm64/whisper-cli");
  });

  it("has no .tar.gz for Windows, which downloads a zip, or for unknown systems", () => {
    expect(tarBuild("win32", "x64")).toBeNull();
    expect(tarBuild("darwin", "ia32")).toBeNull();
    expect(tarBuild("freebsd", "x64")).toBeNull();
  });

  it("pins every download by hash and size", () => {
    for (const [platform, arch] of [["linux", "x64"], ["linux", "arm64"], ["darwin", "arm64"], ["darwin", "x64"]] as const) {
      const build = tarBuild(platform, arch)!;
      expect(build.sha256).toMatch(/^[0-9a-f]{64}$/);
      expect(build.size).toBeGreaterThan(1_000_000);
    }
  });
});
