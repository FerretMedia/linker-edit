import { describe, expect, it } from "vitest";
import { buildFor, CLOUDFLARED_VERSION, readLogLine } from "./cloudflared.ts";

describe("cloudflared", () => {
  it("has a pinned build for every computer Linker-Edit runs on", () => {
    for (const [platform, arch] of [
      ["win32", "x64"],
      ["win32", "arm64"],
      ["darwin", "arm64"],
      ["darwin", "x64"],
      ["linux", "x64"],
      ["linux", "arm64"],
    ] as const) {
      const build = buildFor(platform, arch);
      expect(build?.url).toContain(`/${CLOUDFLARED_VERSION}/`);
      expect(build?.sha256).toMatch(/^[0-9a-f]{64}$/);
    }
    expect(buildFor("linux", "ia32")).toBeNull();
  });

  it("reads the tunnel's state from its log", () => {
    expect(readLogLine('2026-11-16T10:00:00Z INF Registered tunnel connection connIndex=0 location=ams01 protocol=quic')).toEqual({
      state: "up",
    });
    expect(readLogLine("2026-11-16T10:00:00Z WRN Retrying connection in up to 2s")).toEqual({ state: "connecting" });
    expect(readLogLine("2026-11-16T10:00:00Z INF Unregistered tunnel connection connIndex=0 event=0 ip=198.41.192.7")).toEqual({
      state: "connecting",
    });
    expect(readLogLine("2026-11-16T10:00:00Z INF Starting tunnel tunnelID=abc")).toBeNull();
  });
});
