import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { createSessions, SESSION_LIFETIME } from "./sessions.ts";

let file: string;
let clock: number;
beforeEach(() => {
  file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-sessions-")), "connect-sessions.json");
  clock = 1_000_000;
});
const make = () => createSessions({ file, now: () => clock });

describe("Linker Connect sessions", () => {
  it("swaps a code for a session once", () => {
    const sessions = make();
    const code = sessions.issueCode("Laptop");
    const entered = sessions.enter(code);
    expect(entered).not.toBeNull();
    expect(sessions.enter(code)).toBeNull();
    expect(sessions.check(entered!.token)).toEqual(expect.any(String));
    expect(sessions.list()).toEqual([expect.objectContaining({ name: "Laptop", kind: "cookie" })]);
  });

  it("takes a session only the way it was given out", () => {
    const sessions = make();
    const native = sessions.enter(sessions.issueCode("iPad"), "native")!;
    expect(native.expiresAt).toBe(clock + SESSION_LIFETIME);
    expect(sessions.check(native.token, "native")).toEqual(expect.any(String));
    expect(sessions.check(native.token, "cookie")).toBeNull();
    const cookie = sessions.enter(sessions.issueCode("Laptop"))!;
    expect(sessions.check(cookie.token, "native")).toBeNull();
    expect(sessions.list().map((device) => [device.name, device.kind])).toEqual([
      ["iPad", "native"],
      ["Laptop", "cookie"],
    ]);
  });

  it("refuses codes after a minute, and guesses", () => {
    const sessions = make();
    const code = sessions.issueCode("Laptop");
    clock += 61_000;
    expect(sessions.enter(code)).toBeNull();
    expect(sessions.enter("guess")).toBeNull();
    expect(sessions.check("guess")).toBeNull();
    expect(sessions.check(null)).toBeNull();
  });

  it("keeps only hashes, and sessions but not codes across restarts", () => {
    const before = make().issueCode("Laptop");
    expect(make().enter(before)).toBeNull();
    const sessions = make();
    const { token } = sessions.enter(sessions.issueCode("Laptop"))!;
    expect(fs.readFileSync(file, "utf8")).not.toContain(token);
    expect(make().check(token)).toEqual(expect.any(String));
  });

  it("ends sessions after 30 days", () => {
    const sessions = make();
    const { token } = sessions.enter(sessions.issueCode("Laptop"))!;
    const native = sessions.enter(sessions.issueCode("iPad"), "native")!;
    clock += SESSION_LIFETIME + 1;
    expect(sessions.check(token)).toBeNull();
    expect(sessions.check(native.token, "native")).toBeNull();
    expect(sessions.list()).toEqual([]);
  });

  it("signs devices out, and says which", () => {
    const sessions = make();
    const ended: string[][] = [];
    sessions.onRevoke((ids) => ended.push(ids));
    const laptop = sessions.enter(sessions.issueCode("Laptop"))!;
    const office = sessions.enter(sessions.issueCode("Office"))!;
    const laptopId = sessions.check(laptop.token)!;
    sessions.revoke(laptopId);
    expect(sessions.check(laptop.token)).toBeNull();
    expect(sessions.check(office.token)).not.toBeNull();
    const code = sessions.issueCode("Phone");
    sessions.revokeAll();
    expect(sessions.check(office.token)).toBeNull();
    expect(sessions.enter(code)).toBeNull();
    expect(ended).toEqual([[laptopId], [expect.any(String)]]);
  });
});
