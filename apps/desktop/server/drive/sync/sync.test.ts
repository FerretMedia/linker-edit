import { createHash, randomBytes } from "node:crypto";
import * as fs from "node:fs";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { fakeGoogle } from "../../../e2e/fake-google.ts";
import { CHUNK_SIZE, createDriveClient } from "../client.ts";
import { MANIFEST_FILE, syncProject, SyncStopped } from "./sync.ts";

let google: ReturnType<typeof fakeGoogle>;
let url: string;
let folderId: string;
/** Every request a client sent: "METHOD url range", with an upload's Content-Range or a download's Range. */
let requests: string[];

beforeEach(async () => {
  google = fakeGoogle();
  await new Promise<void>((resolve) => google.server.listen(0, "127.0.0.1", resolve));
  url = `http://127.0.0.1:${(google.server.address() as AddressInfo).port}`;
  folderId = google.drive.add({ name: "Launch reel", parent: "root" }).id;
  requests = [];
});
afterEach(() => void google.server.close());

/** A Drive client that records its requests, and may fail one on purpose, the way a killed app stops. */
function client(kill?: (method: string, target: string, range: string) => boolean) {
  return createDriveClient({
    token: async () => "fake-access-test",
    url,
    sleep: async () => {},
    fetch: async (input, init) => {
      const method = init?.method ?? "GET";
      const target = String(input);
      const headers = new Headers(init?.headers);
      const range = headers.get("content-range") ?? headers.get("range") ?? "";
      requests.push(`${method} ${target} ${range}`.trim());
      if (kill?.(method, target, range)) throw new Error("killed");
      return fetch(input, init);
    },
  });
}

function computer(name: string) {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), `linker-edit-sync-${name}-`));
  const write = (file: string, text: string | Buffer) => {
    fs.mkdirSync(path.dirname(path.join(dir, file)), { recursive: true });
    fs.writeFileSync(path.join(dir, file), text);
  };
  const read = (file: string) => fs.readFileSync(path.join(dir, file), "utf8");
  const sync = (drive = client()) => syncProject({ drive, dir, folderId, computer: name });
  return { dir, write, read, sync };
}

/** The files in a project folder that sync, with their md5s. */
function files(dir: string): Record<string, string> {
  const found: Record<string, string> = {};
  for (const entry of fs.readdirSync(dir, { recursive: true, withFileTypes: true })) {
    const file = path.relative(dir, path.join(entry.parentPath, entry.name)).split(path.sep).join("/");
    if (entry.isFile() && !file.startsWith(".linker-edit/") && !file.startsWith(".git/")) found[file] = createHash("md5").update(fs.readFileSync(path.join(dir, file))).digest("hex");
  }
  return found;
}

/** Paths of what is on Drive in the project's folder and not in the bin. */
function onDrive(): string[] {
  const at = (id: string): string | null => {
    if (id === folderId) return "";
    const file = google.drive.files.get(id);
    if (!file || file.trashed) return null;
    const parent = at(file.parents[0]!);
    return parent === null ? null : parent ? `${parent}/${file.name}` : file.name;
  };
  return [...google.drive.files.values()].flatMap((file) => (file.content && at(file.id) !== null ? [at(file.id)!] : [])).sort();
}

const since = (mark: number) => requests.slice(mark);

describe("Drive sync", () => {
  it("brings two computers to the same files through one Drive", async () => {
    const a = computer("A");
    const b = computer("B");
    a.write("media/clip.mov", randomBytes(50_000));
    a.write("linker-edit.json", "{}");
    a.write("notes.txt", "first");
    a.write(".git/HEAD", "ref: refs/heads/main");
    a.write(".linker-edit/threads/t1.json", "{}");
    a.write(".linker-edit/local.json", "{}");

    expect(await a.sync()).toEqual({ changed: [], failures: [], skipped: [] });
    expect(onDrive()).toEqual(["linker-edit.json", "media/clip.mov", "notes.txt"]);
    await b.sync();
    expect(files(b.dir)).toEqual(files(a.dir));
    expect(fs.existsSync(path.join(b.dir, ".git"))).toBe(false);

    a.write("media/clip.mov", randomBytes(60_000));
    b.write("cut.txt", "from B");
    await a.sync();
    await b.sync();
    await a.sync();
    expect(files(a.dir)).toEqual(files(b.dir));
    expect(a.read("cut.txt")).toBe("from B");

    // Changed on both: the computer that syncs second keeps its version as a copy.
    a.write("notes.txt", "A's notes");
    b.write("notes.txt", "B's notes");
    await a.sync();
    await b.sync();
    await a.sync();
    expect(a.read("notes.txt")).toBe("A's notes");
    expect(a.read("notes (from B).txt")).toBe("B's notes");
    expect(files(b.dir)).toEqual(files(a.dir));

    // A rename goes up and comes down without the clip's bytes.
    fs.renameSync(path.join(a.dir, "media", "clip.mov"), path.join(a.dir, "media", "intro.mov"));
    let mark = requests.length;
    await a.sync();
    expect(since(mark).filter((request) => request.includes("uploadType=resumable"))).toEqual([]);
    mark = requests.length;
    await b.sync();
    expect(since(mark).filter((request) => request.includes("alt=media"))).toEqual([]);
    expect(Object.keys(files(b.dir))).toContain("media/intro.mov");

    fs.rmSync(path.join(b.dir, "notes.txt"));
    await b.sync();
    expect(onDrive()).not.toContain("notes.txt");
    a.write(".linker-edit/drive-bin/2000-01-01/old.txt", "long gone");
    expect(await a.sync()).toMatchObject({ changed: ["notes.txt"] });
    expect(fs.existsSync(path.join(a.dir, ".linker-edit", "drive-bin", "2000-01-01"))).toBe(false);
    expect(files(a.dir)).toEqual(files(b.dir));
    // Deleted on Drive: A keeps its copy in the bin, which never syncs.
    const binned = path.join(a.dir, ".linker-edit", "drive-bin", new Date().toISOString().slice(0, 10), "notes.txt");
    expect(fs.readFileSync(binned, "utf8")).toBe("A's notes");
    expect(Object.keys(files(a.dir)).sort()).toEqual(["cut.txt", "linker-edit.json", "media/intro.mov", "notes (from B).txt"]);
    expect(onDrive()).toEqual(["cut.txt", "linker-edit.json", "media/intro.mov", "notes (from B).txt"]);
  });

  it("carries on a killed upload from where Drive says it got to", async () => {
    const a = computer("A");
    const bytes = randomBytes(CHUNK_SIZE + 1000);
    a.write("clip.mov", bytes);

    const killed = await a.sync(client((method, target, range) => method === "PUT" && range.startsWith(`bytes ${CHUNK_SIZE}-`)));
    expect(killed.failures).toEqual([{ path: "clip.mov", message: "killed" }]);
    expect(onDrive()).toEqual([]);

    const mark = requests.length;
    expect((await a.sync()).failures).toEqual([]);
    const session = JSON.parse(fs.readFileSync(path.join(a.dir, MANIFEST_FILE), "utf8")) as { uploads: object };
    expect(session.uploads).toEqual({});
    expect(since(mark).filter((request) => request.startsWith("PUT") || request.includes("uploadType=resumable"))).toEqual([
      expect.stringMatching(new RegExp(`/upload/session/\\w+ bytes \\*/${bytes.length}$`)),
      expect.stringMatching(new RegExp(`/upload/session/\\w+ bytes ${CHUNK_SIZE}-${bytes.length - 1}/${bytes.length}$`)),
    ]);
    expect([...google.drive.files.values()].find((file) => file.name === "clip.mov")?.content?.equals(bytes)).toBe(true);
  });

  it("carries on a killed download, and puts it in place only when its md5 matches", async () => {
    const bytes = randomBytes(300_000);
    google.drive.add({ name: "clip.mov", parent: folderId, content: bytes });
    const b = computer("B");
    const killAfter = (have: number) =>
      createDriveClient({
        token: async () => "fake-access-test",
        url,
        sleep: async () => {},
        fetch: async (input, init) => {
          requests.push(`${init?.method ?? "GET"} ${String(input)} ${new Headers(init?.headers).get("range") ?? ""}`.trim());
          const res = await fetch(input, init);
          if (!String(input).includes("alt=media")) return res;
          const reader = res.body!.getReader();
          let sent = 0;
          const body = new ReadableStream<Uint8Array>({
            async pull(controller) {
              const { value } = await reader.read();
              if (sent >= have || !value) return controller.error(new Error("killed"));
              const part = value.subarray(0, have - sent);
              sent += part.length;
              controller.enqueue(part);
            },
          });
          return new Response(body, { status: res.status, headers: res.headers });
        },
      });

    expect((await b.sync(killAfter(100_000))).failures).toEqual([{ path: "clip.mov", message: "killed" }]);
    expect(fs.existsSync(path.join(b.dir, "clip.mov"))).toBe(false);

    // What arrived is spoiled before the next pass carries on from it.
    const [part] = fs.readdirSync(path.join(b.dir, ".linker-edit", "drive-downloads"));
    const partFile = path.join(b.dir, ".linker-edit", "drive-downloads", part!);
    expect(fs.statSync(partFile).size).toBe(100_000);
    fs.writeFileSync(partFile, Buffer.alloc(100_000));
    const mark = requests.length;
    const spoiled = await b.sync();
    expect(since(mark).filter((request) => request.includes("alt=media"))).toEqual([expect.stringMatching(/alt=media bytes=100000-$/)]);
    expect(spoiled.failures).toEqual([{ path: "clip.mov", message: "It didn't arrive whole from Google Drive. It downloads again next time." }]);
    expect(fs.existsSync(path.join(b.dir, "clip.mov"))).toBe(false);

    expect(await b.sync()).toMatchObject({ changed: ["clip.mov"], failures: [] });
    expect(fs.readFileSync(path.join(b.dir, "clip.mov")).equals(bytes)).toBe(true);
  });

  it("stops, deleting nothing, when the project's folder is in Drive's bin", async () => {
    const a = computer("A");
    a.write("clip.mov", "bytes");
    await a.sync();
    google.drive.files.get(folderId)!.trashed = true;

    await expect(a.sync()).rejects.toBeInstanceOf(SyncStopped);
    expect(a.read("clip.mov")).toBe("bytes");
  });
});
