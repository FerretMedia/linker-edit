// A stand-in for Google as far as signing in to Drive and the Drive sync go. Its consent screen sends the browser
// straight back with a code, its token endpoint checks the PKCE verifier, and Drive's about.get says who signed in.
// Its Drive keeps files in memory: folders, resumable uploads, ranged downloads and the changes feed. The e2e
// server's googleUrl points at it, and the server/drive tests run it on a port of their own.
//
//   node e2e/fake-google.ts [port]
import { createHash, randomBytes } from "node:crypto";
import * as http from "node:http";

export const FAKE_GOOGLE_PORT = 4397;
export const FAKE_GOOGLE_CLIENT = { id: "fake-google-client", secret: "fake-google-secret" };
export const FAKE_GOOGLE_USER = { permissionId: "07362580457318461234", emailAddress: "jane@drive.example", displayName: "Jane Doe" };
/** A project on the e2e Drive, with one clip that downloads at E2E_DRIVE_SPEED. */
export const E2E_DRIVE_PROJECT = { folderId: "e2e-launch-reel", clip: "clip.mov", clipSize: 4 * 1024 * 1024 };
const E2E_DRIVE_SPEED = 2 * 1024 * 1024;
const SCOPE = "https://www.googleapis.com/auth/drive.file";
const FOLDER = "application/vnd.google-apps.folder";

interface FakeFile {
  id: string;
  name: string;
  mimeType: string;
  parents: string[];
  /** Null for a folder. */
  content: Buffer | null;
  modifiedTime: string;
  trashed: boolean;
}

type Session = { size: number; have: Buffer[]; received: number } & ({ name: string; parent: string } | { fileId: string });

const md5 = (bytes: Buffer) => createHash("md5").update(bytes).digest("hex");

/** Drive's files, changes feed and uploads, for requests a fake Google passes on. */
function fakeDrive(bytesPerSecond: number | null) {
  const files = new Map<string, FakeFile>();
  const changed = new Map<string, number>();
  const sessions = new Map<string, Session>();
  let seq = 0;
  let ids = 0;

  function touch(file: FakeFile) {
    file.modifiedTime = new Date().toISOString();
    changed.set(file.id, ++seq);
    return file;
  }

  function add(fields: { id?: string; name: string; parent: string; content?: Buffer | null }): FakeFile {
    const id = fields.id ?? `file-${++ids}`;
    const content = fields.content ?? null;
    const file: FakeFile = { id, name: fields.name, parents: [fields.parent], mimeType: content ? "application/octet-stream" : FOLDER, content, modifiedTime: "", trashed: false };
    files.set(id, file);
    return touch(file);
  }

  const resource = (file: FakeFile) => ({
    id: file.id,
    name: file.name,
    mimeType: file.mimeType,
    parents: file.parents,
    modifiedTime: file.modifiedTime,
    trashed: file.trashed,
    ...(file.content ? { md5Checksum: md5(file.content), size: String(file.content.length) } : {}),
  });

  /** The parts of Drive's search syntax the sync uses: "'id' in parents", "trashed = false", "name = '…'", "mimeType = '…'". */
  function matches(file: FakeFile, q: string): boolean {
    return q.split(" and ").every((clause) => {
      const parent = /^'(.+)' in parents$/.exec(clause);
      if (parent) return file.parents.includes(parent[1]!);
      if (clause === "trashed = false") return !file.trashed;
      const field = /^(name|mimeType) = '(.*)'$/.exec(clause);
      if (field) return file[field[1] as "name" | "mimeType"] === field[2]!.replace(/\\(.)/g, "$1");
      throw new Error(`The fake Drive can't search for ${clause}`);
    });
  }

  function finish(session: Session): FakeFile {
    const content = Buffer.concat(session.have);
    if ("fileId" in session) {
      const file = files.get(session.fileId)!;
      file.content = content;
      return touch(file);
    }
    return add({ name: session.name, parent: session.parent, content });
  }

  async function send(res: http.ServerResponse, status: number, bytes: Buffer) {
    res.writeHead(status, { "content-type": "application/octet-stream", "content-length": bytes.length });
    if (!bytesPerSecond) return void res.end(bytes);
    const piece = 64 * 1024;
    for (let at = 0; at < bytes.length && !res.destroyed; at += piece) {
      res.write(bytes.subarray(at, at + piece));
      await new Promise((resolve) => setTimeout(resolve, (piece / bytesPerSecond) * 1000));
    }
    res.end();
  }

  /** Answers a request to Drive; false when it isn't one. */
  function handle(req: http.IncomingMessage, res: http.ServerResponse, url: URL, body: Buffer): boolean {
    const answer = (status: number, value: unknown) => {
      res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(value));
      return true;
    };
    const notFound = () => answer(404, { error: { code: 404, message: "File not found", errors: [{ reason: "notFound" }] } });
    const json = () => (body.length ? (JSON.parse(body.toString()) as Record<string, unknown>) : {});
    const { pathname } = url;
    const sessionPath = /^\/upload\/session\/(\w+)$/.exec(pathname);
    if (sessionPath) {
      const session = sessions.get(sessionPath[1]!);
      if (!session) return notFound();
      const range = req.headers["content-range"] ?? "";
      const chunk = /^bytes (\d+)-(\d+)\/(\d+)$/.exec(range);
      if (chunk) {
        if (Number(chunk[1]) !== session.received) return answer(400, { error: { message: "Wrong offset" } });
        session.have.push(body);
        session.received += body.length;
      }
      if (session.received === session.size) {
        sessions.delete(sessionPath[1]!);
        return answer(200, resource(finish(session)));
      }
      res.writeHead(308, session.received ? { range: `bytes=0-${session.received - 1}` } : {}).end();
      return true;
    }
    if (!pathname.startsWith("/drive/v3/") && !pathname.startsWith("/upload/drive/v3/")) return false;
    if (!req.headers.authorization?.startsWith("Bearer fake-access-")) return answer(401, { error: { code: 401, message: "Invalid Credentials" } });

    const route = `${req.method} ${pathname.replace(/\/files\/[^/]+$/, "/files/:id")}`;
    const id = decodeURIComponent(pathname.split("/").pop()!);
    const file = files.get(id);
    switch (route) {
      case "GET /drive/v3/files":
        return answer(200, { files: [...files.values()].filter((each) => matches(each, url.searchParams.get("q") ?? "")).map(resource) });
      case "POST /drive/v3/files": {
        const fields = json() as { name: string; parents: string[] };
        return answer(200, resource(add({ name: fields.name, parent: fields.parents[0]! })));
      }
      case "GET /drive/v3/files/:id": {
        if (!file) return notFound();
        if (url.searchParams.get("alt") !== "media") return answer(200, resource(file));
        const from = Number(/^bytes=(\d+)-$/.exec(req.headers.range ?? "")?.[1] ?? 0);
        void send(res, from ? 206 : 200, file.content!.subarray(from));
        return true;
      }
      case "PATCH /drive/v3/files/:id": {
        if (!file) return notFound();
        const fields = json() as { name?: string; trashed?: boolean };
        const from = url.searchParams.get("removeParents");
        const to = url.searchParams.get("addParents");
        if (from && to) file.parents = file.parents.map((parent) => (parent === from ? to : parent));
        if (fields.name !== undefined) file.name = fields.name;
        if (fields.trashed !== undefined) file.trashed = fields.trashed;
        return answer(200, resource(touch(file)));
      }
      case "POST /upload/drive/v3/files":
      case "PATCH /upload/drive/v3/files/:id": {
        if (req.method === "PATCH" && !file) return notFound();
        const fields = json() as { name?: string; parents?: string[] };
        const key = randomBytes(6).toString("hex");
        const size = Number(req.headers["x-upload-content-length"]);
        sessions.set(key, { size, have: [], received: 0, ...(file ? { fileId: file.id } : { name: fields.name!, parent: fields.parents![0]! }) });
        res.writeHead(200, { location: `http://${req.headers.host}/upload/session/${key}` }).end();
        return true;
      }
      case "GET /drive/v3/changes/startPageToken":
        return answer(200, { startPageToken: String(seq + 1) });
      case "GET /drive/v3/changes": {
        const since = Number(url.searchParams.get("pageToken"));
        const list = [...changed].filter(([, at]) => at >= since).sort((a, b) => a[1] - b[1]);
        return answer(200, { changes: list.map(([fileId]) => ({ fileId, removed: false, file: resource(files.get(fileId)!) })), newStartPageToken: String(seq + 1) });
      }
      default:
        return false;
    }
  }

  return { files, add, handle };
}

export type FakeDrive = ReturnType<typeof fakeDrive>;

/** Google's endpoints at their usual paths; calls lists each request as "METHOD /path". */
export function fakeGoogle(options: { bytesPerSecond?: number } = {}) {
  const calls: string[] = [];
  const codes = new Map<string, { challenge: string; redirectUri: string }>();
  const refreshTokens = new Set<string>();
  const drive = fakeDrive(options.bytesPerSecond ?? null);
  let issued = 0;
  const server = http.createServer((req, res) => {
    const url = new URL(req.url ?? "/", "http://127.0.0.1");
    const route = `${req.method} ${url.pathname}`;
    calls.push(route);
    const answer = (status: number, body: unknown) => res.writeHead(status, { "content-type": "application/json" }).end(JSON.stringify(body));
    const tokens = () => {
      issued += 1;
      return { access_token: `fake-access-${issued}`, expires_in: 3600, scope: SCOPE, token_type: "Bearer" };
    };
    const chunks: Buffer[] = [];
    req.on("data", (chunk: Buffer) => chunks.push(chunk));
    req.on("end", () => {
      const body = Buffer.concat(chunks);
      const form = new URLSearchParams(body.toString());
      switch (route) {
        case "GET /o/oauth2/v2/auth": {
          const query = url.searchParams;
          if (query.get("client_id") !== FAKE_GOOGLE_CLIENT.id || query.get("code_challenge_method") !== "S256") return answer(400, { error: "invalid_request" });
          const code = randomBytes(8).toString("hex");
          codes.set(code, { challenge: query.get("code_challenge") ?? "", redirectUri: query.get("redirect_uri") ?? "" });
          const back = new URL(query.get("redirect_uri") ?? "");
          back.search = new URLSearchParams({ code, state: query.get("state") ?? "", scope: query.get("scope") ?? "" }).toString();
          return res.writeHead(302, { location: back.toString() }).end();
        }
        case "POST /token": {
          if (form.get("client_id") !== FAKE_GOOGLE_CLIENT.id || form.get("client_secret") !== FAKE_GOOGLE_CLIENT.secret) {
            return answer(401, { error: "invalid_client" });
          }
          if (form.get("grant_type") === "refresh_token") {
            return refreshTokens.has(form.get("refresh_token") ?? "") ? answer(200, tokens()) : answer(400, { error: "invalid_grant" });
          }
          const asked = codes.get(form.get("code") ?? "");
          codes.delete(form.get("code") ?? "");
          const verifier = form.get("code_verifier") ?? "";
          if (!asked || asked.redirectUri !== form.get("redirect_uri") || createHash("sha256").update(verifier).digest("base64url") !== asked.challenge) {
            return answer(400, { error: "invalid_grant" });
          }
          const refreshToken = `fake-refresh-${randomBytes(4).toString("hex")}`;
          refreshTokens.add(refreshToken);
          return answer(200, { ...tokens(), refresh_token: refreshToken });
        }
        case "POST /revoke":
          refreshTokens.delete(form.get("token") ?? "");
          return answer(200, {});
        case "GET /drive/v3/about":
          return req.headers.authorization?.startsWith("Bearer fake-access-")
            ? answer(200, { user: FAKE_GOOGLE_USER })
            : answer(401, { error: { code: 401, message: "Invalid Credentials" } });
        case "GET /health":
          return answer(200, { ok: true });
        default:
          if (!drive.handle(req, res, url, body)) answer(404, { error: "not_found" });
      }
    });
  });
  return { server, calls, drive };
}

if (import.meta.main) {
  const port = Number(process.argv[2] ?? FAKE_GOOGLE_PORT);
  const google = fakeGoogle({ bytesPerSecond: E2E_DRIVE_SPEED });
  google.drive.add({ id: E2E_DRIVE_PROJECT.folderId, name: "Launch reel", parent: "root" });
  google.drive.add({ name: E2E_DRIVE_PROJECT.clip, parent: E2E_DRIVE_PROJECT.folderId, content: Buffer.alloc(E2E_DRIVE_PROJECT.clipSize, 7) });
  google.server.listen(port, "127.0.0.1", () => console.log(`Fake Google on http://127.0.0.1:${port}`));
}
