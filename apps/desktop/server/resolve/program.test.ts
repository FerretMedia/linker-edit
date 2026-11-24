import { describe, expect, it } from "vitest";
import { launchCommand, launchEnv, launchResolve, parseProcesses } from "./program.ts";

const WINDOWS = [
  '"System Idle Process","0","Services","0","8 K"',
  '"ResolveMCP.exe","14564","Console","1","3,776 K"',
  '"DaVinci Resolve Welcome.exe","14600","Console","1","3,776 K"',
];
const MAC = [
  "/sbin/launchd",
  "/Applications/DaVinci Resolve/DaVinci Resolve.app/Contents/Applications/ResolveMCP",
  "/System/Library/CoreServices/Finder.app/Contents/MacOS/Finder",
];
const MAC_RESOLVE = "/Applications/DaVinci Resolve/DaVinci Resolve.app/Contents/MacOS/Resolve";
const MAC_SCRIPT_SERVER = "/Applications/DaVinci Resolve/DaVinci Resolve.app/Contents/Libraries/Fusion/fuscript";
const LINUX = ["systemd", "kworker/0:1", "ResolveMCP", "fuscript <defunct>"];

describe("parseProcesses", () => {
  it.each([
    ["win32", WINDOWS, '"Resolve.exe","900","Console","1","2,000,000 K"', '"fuscript.exe","901","Console","1","9,000 K"', "\r\n"],
    ["darwin", MAC, MAC_RESOLVE, MAC_SCRIPT_SERVER, "\n"],
    ["linux", LINUX, "resolve", "fuscript", "\n"],
  ] as const)("finds Resolve and its script server on %s", (platform, others, resolve, scriptServer, newline) => {
    const parse = (...lines: string[]) => parseProcesses([...others, ...lines, ""].join(newline), platform);
    expect(parse()).toEqual({ resolve: false, scriptServer: false });
    expect(parse(resolve)).toEqual({ resolve: true, scriptServer: false });
    expect(parse(resolve, scriptServer)).toEqual({ resolve: true, scriptServer: true });
  });

  it.each([
    ["win32", ""],
    ["win32", "INFO: No tasks are running which match the specified criteria.\r\n"],
    ["darwin", "\n"],
    ["linux", ""],
  ] as const)("doesn't take a %s listing without processes (%j) for a closed Resolve", (platform, output) => {
    expect(() => parseProcesses(output, platform)).toThrow();
  });
});

describe("launching Resolve", () => {
  it("goes through a cmd that ends, on Windows", () => {
    expect(launchCommand("D:\\My Programs\\DaVinci Resolve\\ResolveMCP.exe", "win32")).toEqual({
      program: "D:\\My Programs\\DaVinci Resolve\\Resolve.exe",
      command: "cmd.exe",
      args: ["/d", "/s", "/c", '"start "" "D:\\My Programs\\DaVinci Resolve\\Resolve.exe""'],
    });
  });

  it("opens the app ResolveMCP is in, on macOS", () => {
    const program = "/Applications/DaVinci Resolve/DaVinci Resolve.app";
    expect(launchCommand(`${program}/Contents/Applications/ResolveMCP`, "darwin")).toEqual({ program, command: "open", args: [program] });
  });

  it("runs resolve next to ResolveMCP, on Linux", () => {
    const program = "/mnt/my disk/resolve/bin/resolve";
    expect(launchCommand("/mnt/my disk/resolve/bin/ResolveMCP", "linux")).toEqual({ program, command: program, args: [] });
  });

  it("keeps the server's own variables from Resolve", () => {
    expect(launchEnv({ Path: "C:\\Windows", ELECTRON_RUN_AS_NODE: "1", LINKER_EDIT_DESKTOP: "1", LINKER_EDIT_CONFIG: "x.json" })).toEqual({
      Path: "C:\\Windows",
    });
  });

  it("says so when Resolve isn't where its MCP server is", async () => {
    await expect(launchResolve("/no such folder/a/b/ResolveMCP")).rejects.toMatchObject({
      kind: "unavailable",
      message: expect.stringContaining("is missing"),
    });
  });
});
