import { createHash } from "node:crypto";
import * as http from "node:http";
import type { IncomingMessage, ServerResponse } from "node:http";
import type { AddressInfo } from "node:net";
import { crc32, deflateRawSync } from "node:zlib";

/** A zip as zip tools write one; `stored` leaves the files uncompressed. */
export function zip(files: Record<string, string | Uint8Array>, options: { stored?: boolean; claimedSize?: number } = {}): Uint8Array {
  const locals: Buffer[] = [];
  const entries: Buffer[] = [];
  let offset = 0;
  for (const [path, content] of Object.entries(files)) {
    const data = Buffer.from(content);
    const packed = options.stored ? data : deflateRawSync(data);
    const name = Buffer.from(path);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(options.stored ? 0 : 8, 8);
    local.writeUInt32LE(crc32(data), 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE(options.stored ? 0 : 8, 10);
    entry.writeUInt32LE(crc32(data), 16);
    entry.writeUInt32LE(packed.length, 20);
    entry.writeUInt32LE(options.claimedSize ?? data.length, 24);
    entry.writeUInt16LE(name.length, 28);
    entry.writeUInt32LE(offset, 42);
    locals.push(local, name, packed);
    entries.push(entry, name);
    offset += local.length + name.length + packed.length;
  }
  const directory = Buffer.concat(entries);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(files).length, 8);
  end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return new Uint8Array(Buffer.concat([...locals, directory, end]));
}

type Role = "admin" | "editor" | "viewer";

/**
 * Linker's skills API (/api/edit/skills) as far as Linker-Edit uses it, kept in memory, for tests and the e2e fake
 * Linker. roles gives each person's role in each org; requests are "<method> <status>" as answered.
 */
export function fakeLinkerSkills(roles: Record<string, Record<string, Role>> = {}) {
  const kept = new Map<string, { holder: string; name: string; files: string; revision: number; updatedAt: string }>();
  const state = { unavailable: false, roles, requests: [] as string[] };
  let clock = Date.UTC(2026, 0, 1);

  const own = (person: string) => `person:${person}`;
  const skillOf = (person: string, row: { holder: string; name: string; files: string; revision: number; updatedAt: string }) => ({
    owner: row.holder === own(person) ? "me" : row.holder,
    name: row.name,
    files: JSON.parse(row.files) as Record<string, string>,
    updatedAt: row.updatedAt,
    revision: row.revision,
  });

  function refusal(files: unknown): { status: number; body: Record<string, string> } | null {
    if (!files || typeof files !== "object" || Array.isArray(files)) return { status: 400, body: { error: "invalid_skill", message: "files must map each path to its text." } };
    const entries = Object.entries(files);
    if (entries.length > 64) return { status: 413, body: { error: "too_large", limit: "files" } };
    for (const [file, text] of entries) {
      if (!file.split("/").every((part) => /^[A-Za-z0-9._-]+$/.test(part) && part !== "." && part !== "..")) {
        return { status: 400, body: { error: "invalid_skill", message: `${file} isn’t a safe relative path.` } };
      }
      // Control characters other than tab, newline, form feed and return aren't text.
      if (typeof text !== "string" || /[\u0000-\u0008\u000B\u000E-\u001F]/.test(text)) return { status: 400, body: { error: "invalid_skill", message: `${file} isn’t text.` } };
    }
    if (!("SKILL.md" in files)) return { status: 400, body: { error: "invalid_skill", message: "A skill needs a SKILL.md." } };
    const bytes = entries.reduce((sum, [, text]) => sum + Buffer.byteLength(text as string), 0);
    return bytes > 512 * 1024 ? { status: 413, body: { error: "too_large", limit: "bytes" } } : null;
  }

  /** Answers a request to /api/edit/skills for person (null: no valid token); false for any other path. */
  async function handle(req: IncomingMessage, res: ServerResponse, person: string | null): Promise<boolean> {
    const match = /^\/api\/edit\/skills(?:\/([^/]+)\/([^/]+))?$/.exec(new URL(req.url ?? "/", "http://fake").pathname);
    if (!match) return false;
    const answer = (status: number, body?: unknown, headers: Record<string, string> = {}) => {
      state.requests.push(`${req.method} ${status}`);
      res.writeHead(status, { "content-type": "application/json", ...headers }).end(body === undefined ? undefined : JSON.stringify(body));
      return true;
    };
    if (!person) return answer(401, { error: "unauthorized" });
    if (state.unavailable) return answer(503, { error: "unavailable" });
    const orgs = state.roles[person] ?? {};
    if (!match[1]) {
      const skills = [...kept.values()].filter((row) => row.holder === own(person) || row.holder in orgs).map((row) => skillOf(person, row));
      const etag = `"${createHash("sha256").update(JSON.stringify(skills.map((skill) => [skill.owner, skill.name, skill.revision, skill.updatedAt]))).digest("hex")}"`;
      return req.headers["if-none-match"] === etag ? answer(304, undefined, { etag }) : answer(200, { skills }, { etag });
    }
    const owner = decodeURIComponent(match[1]);
    const name = decodeURIComponent(match[2]!);
    if (owner !== "me" && !orgs[owner]) return answer(403, { error: "not_member" });
    if (owner !== "me" && orgs[owner] === "viewer") return answer(403, { error: "forbidden" });
    const key = `${owner === "me" ? own(person) : owner}\t${name}`;
    const row = kept.get(key);
    if (req.method === "DELETE") {
      if (!row) return answer(404, { error: "not_found" });
      kept.delete(key);
      return answer(204);
    }
    let body = "";
    for await (const chunk of req) body += String(chunk);
    const files = (JSON.parse(body) as { files?: unknown }).files;
    const refused = refusal(files);
    if (refused) return answer(refused.status, refused.body);
    const text = JSON.stringify(Object.fromEntries(Object.entries(files as object).sort(([a], [b]) => (a < b ? -1 : 1))));
    if (row?.files === text) return answer(200, { skill: skillOf(person, row) });
    const ifMatch = req.headers["if-match"];
    if (ifMatch !== undefined && Number(ifMatch) !== (row?.revision ?? 0)) return answer(409, { error: "conflict" });
    const next = { holder: key.split("\t")[0]!, name, files: text, revision: (row?.revision ?? 0) + 1, updatedAt: new Date((clock += 1000)).toISOString() };
    kept.set(key, next);
    return answer(200, { skill: skillOf(person, next) });
  }

  return { state, handle };
}

/** fakeLinkerSkills on a port of its own, where a token is the person's id. */
export async function serveFakeLinkerSkills(roles: Record<string, Record<string, Role>> = {}) {
  const fake = fakeLinkerSkills(roles);
  const server = http.createServer((req, res) => {
    const person = req.headers.authorization?.replace(/^Bearer /, "") || null;
    void fake.handle(req, res, person).then((handled) => handled || res.writeHead(404).end());
  });
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { ...fake, url: `http://127.0.0.1:${(server.address() as AddressInfo).port}`, close: () => server.close() };
}
