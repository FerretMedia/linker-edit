import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { createLinkerSkills } from "./linkerSkills.ts";
import { createSkillSync, decide, hashFiles, type Action, type Agreed } from "./sync.ts";
import { fakeLinkerSkills } from "./testing.ts";

const A = hashFiles({ "SKILL.md": "A" });
const B = hashFiles({ "SKILL.md": "B" });
const C = hashFiles({ "SKILL.md": "C" });
const at = (revision: number, hash: string): Agreed => ({ revision, hash });

describe("what a pass does about one skill", () => {
  // [this computer's copy, the base, Linker's copy, what is done]
  const cases: [string, string | null, Agreed | undefined, Agreed | null, Action][] = [
    ["same, same", A, at(1, A), at(1, A), "keep"],
    ["same, changed on Linker", A, at(1, A), at(2, B), "pull"],
    ["same, removed on Linker", A, at(1, A), null, "remove_local"],
    ["changed here, same", B, at(1, A), at(1, A), "push"],
    ["changed here, removed on Linker", B, at(1, A), null, "push"],
    ["removed here, same", null, at(1, A), at(1, A), "delete_remote"],
    ["removed here, changed on Linker", null, at(1, A), at(2, B), "pull"],
    ["removed on both", null, at(1, A), null, "forget"],
    ["changed on both", B, at(1, A), at(2, C), "remote_wins"],
    ["changed on both the same way", B, at(1, A), at(2, B), "adopt"],
    ["remade on Linker at revision 1", A, at(1, A), at(1, B), "pull"],
    ["new here", A, undefined, null, "push"],
    ["new on Linker", null, undefined, at(1, A), "pull"],
    ["new on both", A, undefined, at(1, B), "remote_wins"],
    ["new on both, the same", A, undefined, at(3, A), "adopt"],
    ["a folder here without a SKILL.md", null, undefined, null, "keep"],
  ];
  it.each(cases)("%s", (_, local, base, remote, action) => {
    expect(decide(local, base, remote)).toBe(action);
  });
});

const servers: http.Server[] = [];
afterEach(() => {
  for (const server of servers.splice(0)) server.close();
});

/** Linker's skills API on a port of its own; a token is the person's id. */
async function linker(roles: Parameters<typeof fakeLinkerSkills>[0] = {}) {
  const fake = fakeLinkerSkills(roles);
  const server = http.createServer((req, res) => {
    const person = req.headers.authorization?.replace(/^Bearer /, "") || null;
    void fake.handle(req, res, person).then((handled) => handled || res.writeHead(404).end());
  });
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { ...fake, url: `http://127.0.0.1:${(server.address() as AddressInfo).port}` };
}

/** A computer signed in as person, whose own skills are in <root>/skills. */
function computer(url: string, person: () => string | null) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-sync-"));
  const dir = path.join(root, "skills");
  const sync = createSkillSync({
    api: createLinkerSkills({ appUrl: url, accessToken: async () => person() }),
    account: person,
    folders: () => [{ dir, owner: "me", name: "your own skills" }],
    bin: path.join(root, "skills-bin"),
    changed: () => {},
  });
  const write = (name: string, files: Record<string, string>) => {
    for (const [file, text] of Object.entries(files)) {
      fs.mkdirSync(path.dirname(path.join(dir, name, file)), { recursive: true });
      fs.writeFileSync(path.join(dir, name, file), text);
    }
  };
  const read = (name: string) => (fs.existsSync(path.join(dir, name, "SKILL.md")) ? fs.readFileSync(path.join(dir, name, "SKILL.md"), "utf8") : null);
  return { root, dir, sync, write, read };
}

describe("a pass", () => {
  it("is safe to run again after stopping anywhere: a lost base is agreed again without writing", async () => {
    const fake = await linker();
    const one = computer(fake.url, () => "user_jane");
    one.write("hooks", { "SKILL.md": "Sell it." });
    await one.sync.sync();
    expect(fake.state.requests).toEqual(["GET 200", "PUT 200"]);

    // As if it stopped after Linker took the skill and before the base was saved.
    fs.rmSync(path.join(one.dir, ".linker-sync.json"));
    await one.sync.sync();
    await one.sync.sync();
    expect(fake.state.requests.slice(2)).toEqual(["GET 200", "GET 304"]);
    expect(one.sync.status(one.dir)).toEqual({ state: "synced", text: "Synced with Linker", skipped: [] });
  });

  it("skips a skill over Linker's limits, or one with a file Linker can't keep, and says why", async () => {
    const fake = await linker();
    const one = computer(fake.url, () => "user_jane");
    one.write("big", { "SKILL.md": "x".repeat(600 * 1024) });
    one.write("spaced", { "SKILL.md": "Fine.", "my notes.md": "A space." });
    one.write("binary", { "SKILL.md": "Fine." });
    fs.writeFileSync(path.join(one.dir, "binary", "logo.png"), Buffer.from([0x89, 0x50, 0xff, 0xfe]));
    await one.sync.sync();
    expect(one.sync.status(one.dir)).toEqual({
      state: "synced",
      text: "Synced with Linker, except 3 skills",
      skipped: [
        { name: "big", reason: "It's over 512 KB, more than Linker keeps." },
        { name: "binary", reason: "logo.png isn't a text file, so Linker can't keep it." },
        { name: "spaced", reason: "my notes.md isn’t a safe relative path." },
      ],
    });
    // Not sent again until it changes.
    await one.sync.sync();
    expect(fake.state.requests).toEqual(["GET 200", "PUT 400", "GET 304"]);
  });

  it("takes another account's skills for none of its own, so signing in as someone else removes nothing", async () => {
    const fake = await linker();
    let person = "user_jane";
    const one = computer(fake.url, () => person);
    one.write("hooks", { "SKILL.md": "Jane's." });
    await one.sync.sync();
    person = "user_bob";
    await one.sync.sync();
    expect(one.read("hooks")).toBe("Jane's.");
    expect(fake.state.requests.at(-1)).toBe("PUT 200");
  });

  it("does nothing while signed out", async () => {
    const fake = await linker();
    const one = computer(fake.url, () => null);
    one.write("hooks", { "SKILL.md": "Sell it." });
    await one.sync.sync();
    expect(fake.state.requests).toEqual([]);
    expect(one.sync.status(one.dir)).toBeNull();
  });
});
