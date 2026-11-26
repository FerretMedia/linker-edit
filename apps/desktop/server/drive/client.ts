import * as fs from "node:fs";

/**
 * A thin client for Google Drive's REST API (v3), with plain fetch: what the Drive sync needs and no more.
 *
 * Calls carry the Google sign-in's access token. A 401 asks for a newer token once and tries again. Drive's rate
 * limits (429, or 403 with a rate-limit reason), its 5xx errors and dropped connections back off exponentially with
 * jitter, capped, then give up.
 */

export const GOOGLE_API = "https://www.googleapis.com";
const FOLDER_TYPE = "application/vnd.google-apps.folder";
/** Drive wants every chunk but the last in multiples of 256 KiB. */
export const CHUNK_SIZE = 8 * 1024 * 1024;
const FILE_FIELDS = "id,name,mimeType,parents,md5Checksum,size,modifiedTime,trashed";
const MAX_RETRIES = 7;
const FIRST_DELAY = 1000;
const MAX_DELAY = 32_000;
const RATE_LIMITS = new Set(["rateLimitExceeded", "userRateLimitExceeded"]);

export interface DriveFile {
  id: string;
  name: string;
  mimeType: string;
  parents: string[];
  /** Null for folders and Google's own documents, which have no bytes of their own. */
  md5: string | null;
  size: number | null;
  modifiedTime: string;
  trashed: boolean;
}

/** A file that went away, or that Linker-Edit can't see any more, has no file. */
export type DriveChange = { fileId: string; removed: true } | { fileId: string; removed: false; file: DriveFile };

export interface DriveUser {
  /** Stable for the account, unlike the email. */
  permissionId: string;
  email: string;
  name: string | null;
}

/** A new file in a folder, or new contents for a file Drive has. */
export type UploadTarget = { name: string; parent: string } | { fileId: string };

/** Drive said no. Callers branch on status: 404 is a file that's gone, or an upload session that ran out. */
export class DriveError extends Error {
  readonly status: number;
  readonly reason: string | null;
  constructor(status: number, reason: string | null, message: string) {
    super(message);
    this.status = status;
    this.reason = reason;
  }
}

export interface DriveClientOptions {
  /** A token for Drive; given the one Drive just turned away, a newer one. Throws when signed out. */
  token: (rejected?: string) => Promise<string>;
  /** Drive's API, or a stand-in for tests. */
  url?: string;
  fetch?: typeof fetch;
  sleep?: (ms: number) => Promise<void>;
  random?: () => number;
}

interface RawFile {
  id: string;
  name: string;
  mimeType: string;
  parents?: string[];
  md5Checksum?: string;
  size?: string;
  modifiedTime: string;
  trashed?: boolean;
}

function parseFile(raw: RawFile): DriveFile {
  return {
    id: raw.id,
    name: raw.name,
    mimeType: raw.mimeType,
    parents: raw.parents ?? [],
    md5: raw.md5Checksum ?? null,
    size: raw.size === undefined ? null : Number(raw.size),
    modifiedTime: raw.modifiedTime,
    trashed: raw.trashed ?? false,
  };
}

async function driveError(res: Response): Promise<DriveError> {
  const body = (await res.json().catch(() => null)) as { error?: { message?: string; errors?: { reason?: string }[] } } | null;
  return new DriveError(res.status, body?.error?.errors?.[0]?.reason ?? null, body?.error?.message ?? `Google Drive answered ${res.status}.`);
}

/** How far a resumable upload got, from Drive's "Range: bytes=0-N". */
function received(res: Response): number {
  const last = /^bytes=0-(\d+)$/.exec(res.headers.get("range") ?? "");
  return last ? Number(last[1]) + 1 : 0;
}

/** fetch rejects with a TypeError when the connection fails, and a body stream errors with one when it drops. */
const dropped = (error: unknown) => error instanceof TypeError;

export function createDriveClient(options: DriveClientOptions) {
  const api = (options.url ?? GOOGLE_API).replace(/\/$/, "");
  const fetchFn = options.fetch ?? fetch;
  const sleep = options.sleep ?? ((ms: number) => new Promise<void>((resolve) => setTimeout(resolve, ms)));
  const random = options.random ?? Math.random;

  const backoff = (failures: number) => sleep(Math.min(MAX_DELAY, FIRST_DELAY * 2 ** failures + random() * FIRST_DELAY));

  async function retryable(res: Response): Promise<boolean> {
    if (res.status === 429 || res.status >= 500) return true;
    return res.status === 403 && RATE_LIMITS.has((await driveError(res.clone())).reason ?? "");
  }

  /** One call, retried as the top of this file says. Answers whatever Drive finally said, error or not. */
  async function send(url: string, init: RequestInit = {}, auth = true): Promise<Response> {
    let token = auth ? await options.token() : null;
    let refreshed = false;
    for (let failures = 0; ; ) {
      const headers = new Headers(init.headers);
      if (token) headers.set("authorization", `Bearer ${token}`);
      let res: Response;
      try {
        res = await fetchFn(url, { ...init, headers });
      } catch (error) {
        if (!dropped(error) || failures === MAX_RETRIES) throw error;
        await backoff(failures++);
        continue;
      }
      if (res.status === 401 && token && !refreshed) {
        refreshed = true;
        await res.body?.cancel();
        token = await options.token(token);
        continue;
      }
      if (failures < MAX_RETRIES && (await retryable(res))) {
        await res.body?.cancel();
        await backoff(failures++);
        continue;
      }
      return res;
    }
  }

  async function call<T>(path: string, params: Record<string, string>, init: RequestInit = {}): Promise<T> {
    const res = await send(`${api}${path}?${new URLSearchParams(params)}`, init);
    if (!res.ok) throw await driveError(res);
    return (await res.json()) as T;
  }

  /** Where Drive has a resumable upload up to, or the file when it has all of it. */
  async function uploaded(session: string, size: number): Promise<number | DriveFile> {
    // The session address is its own credential: no token, which may run out during a long upload.
    const res = await send(session, { method: "PUT", headers: { "content-range": `bytes */${size}` } }, false);
    if (res.status === 308) {
      await res.body?.cancel();
      return received(res);
    }
    if (!res.ok) throw await driveError(res);
    return parseFile((await res.json()) as RawFile);
  }

  return {
    /** Every file matching q, Drive's search query, through every page. */
    async list(q: string): Promise<DriveFile[]> {
      const files: DriveFile[] = [];
      let pageToken: string | undefined;
      do {
        const page = await call<{ files: RawFile[]; nextPageToken?: string }>("/drive/v3/files", {
          q,
          pageSize: "1000",
          fields: `nextPageToken,files(${FILE_FIELDS})`,
          ...(pageToken ? { pageToken } : {}),
        });
        files.push(...page.files.map(parseFile));
        pageToken = page.nextPageToken;
      } while (pageToken);
      return files;
    },

    async get(fileId: string): Promise<DriveFile> {
      return parseFile(await call<RawFile>(`/drive/v3/files/${encodeURIComponent(fileId)}`, { fields: FILE_FIELDS }));
    },

    /** parent is a folder's id, or "root" for My Drive. A dropped connection retried can make it twice. */
    async createFolder(name: string, parent: string): Promise<DriveFile> {
      const raw = await call<RawFile>(
        "/drive/v3/files",
        { fields: FILE_FIELDS },
        { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name, mimeType: FOLDER_TYPE, parents: [parent] }) },
      );
      return parseFile(raw);
    },

    /** The person signed in. */
    async about(): Promise<DriveUser> {
      const { user } = await call<{ user: { permissionId: string; emailAddress: string; displayName?: string } }>("/drive/v3/about", {
        fields: "user(permissionId,emailAddress,displayName)",
      });
      return { permissionId: user.permissionId, email: user.emailAddress, name: user.displayName || null };
    },

    /** Starts a resumable upload of size bytes. Keep the session address it answers to resume after a restart. */
    async startUpload(target: UploadTarget, size: number): Promise<string> {
      const created = "parent" in target;
      const path = created ? "/upload/drive/v3/files" : `/upload/drive/v3/files/${encodeURIComponent(target.fileId)}`;
      const res = await send(`${api}${path}?${new URLSearchParams({ uploadType: "resumable", fields: FILE_FIELDS })}`, {
        method: created ? "POST" : "PATCH",
        headers: { "content-type": "application/json; charset=UTF-8", "x-upload-content-length": String(size) },
        body: JSON.stringify(created ? { name: target.name, parents: [target.parent] } : {}),
      });
      const session = res.headers.get("location");
      if (!res.ok || !session) throw await driveError(res);
      await res.body?.cancel();
      return session;
    },

    /**
     * Sends a file to a session from startUpload, in CHUNK_SIZE chunks. After a failed chunk, and first when resume
     * is set, it asks Drive how far it got and goes on from there. Answers the file as Drive has it, md5 included.
     */
    async upload(session: string, file: string, { resume = false, onProgress }: { resume?: boolean; onProgress?: (sent: number) => void } = {}): Promise<DriveFile> {
      const handle = await fs.promises.open(file, "r");
      try {
        const { size } = await handle.stat();
        let at = resume ? await uploaded(session, size) : 0;
        let failures = 0;
        while (typeof at === "number") {
          const chunk = Buffer.alloc(Math.min(CHUNK_SIZE, size - at));
          await handle.read(chunk, 0, chunk.length, at);
          const range = size === 0 ? "bytes */0" : `bytes ${at}-${at + chunk.length - 1}/${size}`;
          let res: Response | null = null;
          let failure: unknown;
          try {
            res = await fetchFn(session, { method: "PUT", headers: { "content-range": range }, body: chunk });
          } catch (error) {
            if (!dropped(error)) throw error;
            failure = error;
          }
          if (res?.status === 308) {
            await res.body?.cancel();
            at = received(res);
            failures = 0;
            onProgress?.(at);
          } else if (res?.ok) {
            at = parseFile((await res.json()) as RawFile);
          } else if (failures < MAX_RETRIES && (!res || (await retryable(res)))) {
            await res?.body?.cancel();
            await backoff(failures++);
            at = await uploaded(session, size);
          } else {
            throw res ? await driveError(res) : failure;
          }
        }
        return at;
      } finally {
        await handle.close();
      }
    },

    /**
     * Writes a file's bytes from `from` on into dest, at the same place, so a partial download carries on. A dropped
     * connection carries on from what arrived. dest must exist when from isn't 0.
     */
    async download(fileId: string, dest: string, from = 0): Promise<void> {
      const handle = await fs.promises.open(dest, from > 0 ? "r+" : "w");
      try {
        let at = from;
        for (let failures = 0; ; ) {
          const res = await send(`${api}/drive/v3/files/${encodeURIComponent(fileId)}?alt=media`, { headers: at > 0 ? { range: `bytes=${at}-` } : {} });
          if (!res.ok) throw await driveError(res);
          // Drive sent the whole file after all.
          if (res.status === 200) at = 0;
          try {
            for await (const bytes of (res.body ?? []) as AsyncIterable<Uint8Array>) {
              await handle.write(bytes, 0, bytes.length, at);
              at += bytes.length;
            }
            await handle.truncate(at);
            return;
          } catch (error) {
            if (!dropped(error) || failures === MAX_RETRIES) throw error;
            await backoff(failures++);
          }
        }
      } finally {
        await handle.close();
      }
    },

    /** Where Drive's changes feed is now; changes() from here lists what changes after. */
    async startPageToken(): Promise<string> {
      return (await call<{ startPageToken: string }>("/drive/v3/changes/startPageToken", {})).startPageToken;
    },

    /** Every change since pageToken, through every page, and the token to ask from next time. */
    async changes(pageToken: string): Promise<{ changes: DriveChange[]; pageToken: string }> {
      const changes: DriveChange[] = [];
      for (let token = pageToken; ; ) {
        const page = await call<{ changes: { fileId: string; removed?: boolean; file?: RawFile }[]; nextPageToken?: string; newStartPageToken?: string }>(
          "/drive/v3/changes",
          {
            pageToken: token,
            pageSize: "1000",
            includeRemoved: "true",
            spaces: "drive",
            fields: `nextPageToken,newStartPageToken,changes(fileId,removed,file(${FILE_FIELDS}))`,
          },
        );
        for (const change of page.changes) {
          changes.push(
            change.removed || !change.file ? { fileId: change.fileId, removed: true } : { fileId: change.fileId, removed: false, file: parseFile(change.file) },
          );
        }
        if (page.newStartPageToken) return { changes, pageToken: page.newStartPageToken };
        token = page.nextPageToken!;
      }
    },
  };
}
