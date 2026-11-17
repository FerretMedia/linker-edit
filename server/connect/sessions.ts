import { createHash, randomBytes, randomUUID } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import type { ConnectDevice } from "../../shared/protocol.ts";

/**
 * The devices that opened this computer through Linker Connect. Getting in has two steps, so the session
 * itself never travels in an address:
 * 1. Your other Linker-Edit proves it's your Linker account and gets a one-time code (60 seconds).
 * 2. Its window opens /api/connect/enter?code=…, which swaps the code for a session cookie.
 * Only hashes of codes and sessions are kept, in memory and in the file.
 */

const CODE_LIFETIME = 60_000;
export const SESSION_LIFETIME = 30 * 24 * 60 * 60_000;
/** lastSeenAt is written at most this often. */
const TOUCH_EVERY = 60_000;

interface Saved {
  id: string;
  hash: string;
  name: string;
  createdAt: number;
  lastSeenAt: number;
  expiresAt: number;
}

const hash = (secret: string) => createHash("sha256").update(secret).digest("hex");
const secret = () => randomBytes(32).toString("base64url");

export type Sessions = ReturnType<typeof createSessions>;

export function createSessions(options: { file: string; now?: () => number }) {
  const now = options.now ?? Date.now;
  const codes = new Map<string, { name: string; expiresAt: number }>();
  const revokeListeners = new Set<(ids: string[]) => void>();
  const changeListeners = new Set<() => void>();
  let sessions = read();

  function read(): Saved[] {
    try {
      const list = JSON.parse(fs.readFileSync(options.file, "utf8")) as unknown;
      return Array.isArray(list) ? (list as Saved[]).filter((entry) => entry && typeof entry.hash === "string" && entry.expiresAt > now()) : [];
    } catch {
      return [];
    }
  }

  function write() {
    fs.mkdirSync(path.dirname(options.file), { recursive: true });
    fs.writeFileSync(options.file, JSON.stringify(sessions, null, 2), { mode: 0o600 });
  }

  function changed() {
    for (const listener of changeListeners) listener();
  }

  function drop(ids: string[]) {
    if (ids.length === 0) return;
    sessions = sessions.filter((entry) => !ids.includes(entry.id));
    write();
    for (const listener of revokeListeners) listener(ids);
    changed();
  }

  return {
    /** Step 1: a code for the device called name, good once within a minute. */
    issueCode(name: string): string {
      for (const [key, code] of codes) if (code.expiresAt <= now()) codes.delete(key);
      const code = secret();
      codes.set(hash(code), { name: name.slice(0, 64) || "Another computer", expiresAt: now() + CODE_LIFETIME });
      return code;
    },

    /** Step 2: swaps a code for a session; null when it's wrong, used or too old. */
    enter(code: string): { token: string; maxAgeSeconds: number } | null {
      const key = hash(code);
      const pending = codes.get(key);
      codes.delete(key);
      if (!pending || pending.expiresAt <= now()) return null;
      const token = secret();
      const at = now();
      sessions.push({ id: randomUUID(), hash: hash(token), name: pending.name, createdAt: at, lastSeenAt: at, expiresAt: at + SESSION_LIFETIME });
      write();
      changed();
      return { token, maxAgeSeconds: SESSION_LIFETIME / 1000 };
    },

    /** The session id a cookie's token belongs to, or null. */
    check(token: string | null): string | null {
      if (!token) return null;
      const key = hash(token);
      const session = sessions.find((entry) => entry.hash === key);
      if (!session) return null;
      if (session.expiresAt <= now()) {
        drop([session.id]);
        return null;
      }
      if (now() - session.lastSeenAt > TOUCH_EVERY) {
        session.lastSeenAt = now();
        write();
      }
      return session.id;
    },

    list(): ConnectDevice[] {
      return sessions
        .filter((entry) => entry.expiresAt > now())
        .map((entry) => ({
          id: entry.id,
          name: entry.name,
          createdAt: new Date(entry.createdAt).toISOString(),
          lastSeenAt: new Date(entry.lastSeenAt).toISOString(),
        }));
    },

    revoke(id: string) {
      drop(sessions.some((entry) => entry.id === id) ? [id] : []);
    },

    /** Ends every session, and forgets codes not used yet. */
    revokeAll() {
      codes.clear();
      drop(sessions.map((entry) => entry.id));
    },

    /** Called with the ids of sessions that just ended, so their open connections close. */
    onRevoke(listener: (ids: string[]) => void) {
      revokeListeners.add(listener);
    },

    onChange(listener: () => void) {
      changeListeners.add(listener);
    },
  };
}
