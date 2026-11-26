import { createHash, randomBytes } from "node:crypto";
import * as fs from "node:fs";
import * as http from "node:http";
import type { AddressInfo } from "node:net";
import * as os from "node:os";
import * as path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { CHUNK_SIZE, createDriveClient, DriveError } from "./client.ts";

type Handler = (req: http.IncomingMessage, res: http.ServerResponse, url: URL) => void;

const servers: http.Server[] = [];
afterEach(() => {
  for (const server of servers.splice(0)) server.close();
});

/** A stand-in for Drive's API on a port of its own; each test answers its requests. */
async function fakeDrive(handle: Handler): Promise<string> {
  const server = http.createServer((req, res) => handle(req, res, new URL(req.url ?? "/", "http://127.0.0.1")));
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  return `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
}

const json = (res: http.ServerResponse, status: number, body: unknown) =>
  res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));

const raw = (id: string, extra: Record<string, unknown> = {}) => ({ id, name: `${id}.mov`, mimeType: "video/quicktime", modifiedTime: "2026-11-26T10:00:00.000Z", ...extra });

function client(url: string, token: (rejected?: string) => Promise<string> = async () => "token") {
  const delays: number[] = [];
  const drive = createDriveClient({ token, url, sleep: async (ms) => void delays.push(ms), random: () => 0 });
  return { drive, delays };
}

function tempFile(bytes: Buffer): string {
  const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-drive-")), "clip.mov");
  fs.writeFileSync(file, bytes);
  return file;
}

const md5 = (bytes: Buffer) => createHash("md5").update(bytes).digest("hex");

describe("Drive client", () => {
  it("pages through a search and the changes feed", async () => {
    const asked: string[] = [];
    const url = await fakeDrive((_req, res, url) => {
      const page = url.searchParams.get("pageToken");
      asked.push([url.pathname, url.searchParams.get("q"), page].filter(Boolean).join(" "));
      if (url.pathname === "/drive/v3/files") {
        return page ? json(res, 200, { files: [raw("b", { size: "12", md5Checksum: "abc", parents: ["root"] })] }) : json(res, 200, { files: [raw("a")], nextPageToken: "p2" });
      }
      if (url.pathname === "/drive/v3/changes/startPageToken") return json(res, 200, { startPageToken: "100" });
      if (page === "100") return json(res, 200, { changes: [{ fileId: "a", removed: true }], nextPageToken: "101" });
      return json(res, 200, { changes: [{ fileId: "b", removed: false, file: raw("b", { trashed: true }) }], newStartPageToken: "102" });
    });
    const { drive } = client(url);

    const files = await drive.list("'root' in parents");
    expect(files.map((file) => file.id)).toEqual(["a", "b"]);
    expect(files[1]).toMatchObject({ md5: "abc", size: 12, parents: ["root"], trashed: false });
    expect(files[0]).toMatchObject({ md5: null, size: null });

    const changes = await drive.changes(await drive.startPageToken());
    expect(changes.pageToken).toBe("102");
    expect(changes.changes).toEqual([
      { fileId: "a", removed: true },
      { fileId: "b", removed: false, file: expect.objectContaining({ id: "b", trashed: true }) },
    ]);
    expect(asked).toEqual([
      "/drive/v3/files 'root' in parents",
      "/drive/v3/files 'root' in parents p2",
      "/drive/v3/changes/startPageToken",
      "/drive/v3/changes 100",
      "/drive/v3/changes 101",
    ]);
  });

  it("resumes an upload cut off mid-chunk from where Drive says it got to", async () => {
    const bytes = randomBytes(CHUNK_SIZE * 2 + 1234);
    const size = bytes.length;
    const cutAt = CHUNK_SIZE + 3_000_000;
    const got = Buffer.alloc(size);
    let have = 0;
    let cut = false;
    const ranges: string[] = [];
    const url = await fakeDrive((req, res, url) => {
      if (req.method === "POST") {
        expect(url.pathname).toBe("/upload/drive/v3/files");
        return res.writeHead(200, { location: `http://${req.headers.host}/session/1` }).end();
      }
      const range = req.headers["content-range"] ?? "";
      ranges.push(range);
      const done = () => (have === size ? json(res, 200, raw("up", { md5Checksum: md5(got), size: String(size) })) : res.writeHead(308, { range: `bytes=0-${have - 1}` }).end());
      if (range === `bytes */${size}`) return done();
      // Like Drive, it takes bytes only from where it has them up to.
      if (Number(/^bytes (\d+)-/.exec(range)?.[1]) !== have) return json(res, 400, { error: { message: "Wrong offset" } });
      req.on("data", (chunk: Buffer) => {
        if (cut && req.destroyed) return;
        const room = !cut && have + chunk.length > cutAt ? cutAt - have : chunk.length;
        chunk.copy(got, have, 0, room);
        have += room;
        if (room < chunk.length) {
          cut = true;
          req.socket.destroy();
        }
      });
      req.on("end", done);
    });
    const { drive, delays } = client(url);
    const progress: number[] = [];

    const session = await drive.startUpload({ name: "clip.mov", parent: "folder" }, size);
    const file = await drive.upload(session, tempFile(bytes), { onProgress: (sent) => progress.push(sent) });

    expect(got.equals(bytes)).toBe(true);
    expect(file).toMatchObject({ id: "up", md5: md5(bytes), size });
    expect(ranges).toEqual([`bytes 0-${CHUNK_SIZE - 1}/${size}`, `bytes ${CHUNK_SIZE}-${2 * CHUNK_SIZE - 1}/${size}`, `bytes */${size}`, `bytes ${cutAt}-${size - 1}/${size}`]);
    expect(progress).toEqual([CHUNK_SIZE]);
    expect(delays).toEqual([1000]);
  });

  it("downloads from a byte on, and carries on from what arrived after a dropped connection", async () => {
    const bytes = randomBytes(300_000);
    const ranges: string[] = [];
    const url = await fakeDrive((req, res, url) => {
      expect(url.pathname).toBe("/drive/v3/files/clip");
      const range = req.headers.range ?? "";
      ranges.push(range);
      const from = Number(/^bytes=(\d+)-$/.exec(range)?.[1] ?? 0);
      res.writeHead(206, { "content-length": bytes.length - from });
      if (ranges.length > 1) return res.end(bytes.subarray(from));
      res.write(bytes.subarray(from, from + 100_000), () => res.socket?.end());
    });
    const { drive } = client(url);
    const dest = tempFile(bytes.subarray(0, 1000));

    await drive.download("clip", dest, 1000);

    expect(fs.readFileSync(dest).equals(bytes)).toBe(true);
    // How much of the cut response arrives is up to the socket, but the second ask starts after the first.
    expect(ranges).toHaveLength(2);
    expect(ranges[0]).toBe("bytes=1000-");
    const resumedAt = Number(/^bytes=(\d+)-$/.exec(ranges[1]!)?.[1]);
    expect(resumedAt).toBeGreaterThan(1000);
    expect(resumedAt).toBeLessThanOrEqual(101_000);
  });

  it("asks for a newer token once when Drive turns one away", async () => {
    const url = await fakeDrive((req, res) =>
      req.headers.authorization === "Bearer new" ? json(res, 200, raw("clip")) : json(res, 401, { error: { message: "Invalid Credentials" } }),
    );
    const asked: (string | undefined)[] = [];
    const refreshes = client(url, async (rejected) => {
      asked.push(rejected);
      return rejected ? "new" : "old";
    });
    await expect(refreshes.drive.get("clip")).resolves.toMatchObject({ id: "clip" });
    expect(asked).toEqual([undefined, "old"]);

    asked.length = 0;
    const stale = client(url, async (rejected) => {
      asked.push(rejected);
      return "old";
    });
    await expect(stale.drive.get("clip")).rejects.toMatchObject({ status: 401 });
    expect(asked).toEqual([undefined, "old"]);
  });

  it("backs off on rate limits and server errors, capped, and not on other refusals", async () => {
    const answers: (number | string)[] = [];
    const url = await fakeDrive((_req, res) => {
      const next = answers.shift() ?? 200;
      if (next === 200) return json(res, 200, raw("clip"));
      const [status, reason] = typeof next === "number" ? [next, "backendError"] : [403, next];
      json(res, status, { error: { message: "No", errors: [{ reason }] } });
    });

    const limited = client(url);
    answers.push(429, "userRateLimitExceeded", 503);
    await expect(limited.drive.get("clip")).resolves.toMatchObject({ id: "clip" });
    expect(limited.delays).toEqual([1000, 2000, 4000]);

    const refused = client(url);
    answers.push("insufficientFilePermissions");
    const error = await refused.drive.get("clip").catch((error: unknown) => error);
    expect(error).toBeInstanceOf(DriveError);
    expect(error).toMatchObject({ status: 403, reason: "insufficientFilePermissions" });
    expect(refused.delays).toEqual([]);

    const down = client(url);
    answers.push(...Array<number>(8).fill(503));
    await expect(down.drive.get("clip")).rejects.toMatchObject({ status: 503 });
    expect(down.delays).toEqual([1000, 2000, 4000, 8000, 16000, 32000, 32000]);
  });
});
