// Your own skills on every computer you use, and a brand's for everyone in it: each folder that syncs is brought
// together with Linker's copy in passes. A base file in the folder keeps what both sides last agreed each skill was,
// so a pass can tell which side changed since, and PLAN says what to do about it.
import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import type { SkillSync } from "../../shared/protocol.ts";
import { formatBytes, hiddenPart, SKILL_IMPORT_LIMITS } from "../../shared/skillImport.ts";
import type { ImportedSkill } from "./import.ts";
import { removeSkill, SKILL_FILE, SKILL_NAME, writeSkills } from "./skills.ts";

/** A skill's files by their path in its folder, "/" between folders. */
export type SkillFiles = Record<string, string>;

/** A skill as Linker keeps it: owner is "me" for the person's own, else an org's id. */
export interface RemoteSkill {
  owner: string;
  name: string;
  files: SkillFiles;
  revision: number;
}

/** Linker's skills API, /api/edit/skills. Each call throws a SkillsRefusal when Linker says no or can't be reached. */
export interface SkillsApi {
  /** Every skill of the person and their orgs; null when nothing changed since the list that had this etag. */
  list(etag: string | null): Promise<{ etag: string; skills: RemoteSkill[] } | null>;
  /** ifMatch: only over that revision, 0 only while there is none; null over any. */
  put(owner: string, name: string, files: SkillFiles, ifMatch: number | null): Promise<RemoteSkill>;
  remove(owner: string, name: string): Promise<void>;
}

/** Linker's refusal by its code, such as "forbidden"; "offline" when it couldn't be reached at all. */
export class SkillsRefusal extends Error {
  readonly code: string;
  /** Which limit a "too_large" is over. */
  readonly limit: string | null;
  constructor(code: string, message?: string, limit: string | null = null) {
    super(message ?? code);
    this.code = code;
    this.limit = limit;
  }
}

/** A folder of skills that syncs, and whose skills on Linker it holds. */
export interface SyncFolder {
  dir: string;
  owner: string;
  /** As the status line names it: a brand's name. */
  name: string;
}

/** What this computer and Linker last agreed a skill was. */
export interface Agreed {
  revision: number;
  hash: string;
}

export type Action = "keep" | "adopt" | "pull" | "remove_local" | "push" | "delete_remote" | "remote_wins" | "forget";

type SinceBase = "same" | "changed" | "gone";
type NoBase = "new" | "absent";
type Case = `${SinceBase}/${SinceBase}` | Exclude<`${NoBase}/${NoBase}`, "absent/absent">;

/**
 * What a pass does about one skill, by "this computer/Linker", each against the base. With no base (a skill neither
 * side knew the other had) it is new or absent. Where both changed, Linker wins and this computer's copy goes to the
 * bin; a change wins over a delete.
 */
export const PLAN: Record<Case, Action> = {
  "same/same": "keep",
  "same/changed": "pull",
  "same/gone": "remove_local",
  "changed/same": "push",
  "changed/changed": "remote_wins",
  "changed/gone": "push",
  "gone/same": "delete_remote",
  "gone/changed": "pull",
  "gone/gone": "forget",
  "new/absent": "push",
  "absent/new": "pull",
  "new/new": "remote_wins",
};

/** local is the hash of this computer's copy, null without one; the same files on both sides only need agreeing on. */
export function decide(local: string | null, base: Agreed | undefined, remote: Agreed | null): Action {
  if (local !== null && remote && local === remote.hash) return base?.revision === remote.revision && base.hash === remote.hash ? "keep" : "adopt";
  // Neither side has it: a folder here without a SKILL.md.
  if (!base) return local === null && !remote ? "keep" : PLAN[`${local === null ? "absent" : "new"}/${remote ? "new" : "absent"}` as Case];
  const ours = local === null ? "gone" : local === base.hash ? "same" : "changed";
  // A skill deleted and made again on Linker starts again at revision 1, so its files tell too.
  const theirs = remote === null ? "gone" : remote.revision === base.revision && remote.hash === base.hash ? "same" : "changed";
  return PLAN[`${ours}/${theirs}`];
}

/** The same files are always the same hash, in whatever order they were read: Linker orders them the same way. */
export function hashFiles(files: SkillFiles): string {
  const sorted = Object.entries(files).sort(([a], [b]) => (a < b ? -1 : a > b ? 1 : 0));
  return createHash("sha256").update(JSON.stringify(sorted)).digest("hex");
}

const BASE_FILE = ".linker-sync.json";
const UTF8 = new TextDecoder("utf-8", { fatal: true });

interface Base {
  /** Whose Linker account it was agreed with: another account's skills are no base for this one's. */
  account: string;
  skills: Record<string, Agreed>;
}

function readBase(dir: string, account: string): Base {
  try {
    const saved = JSON.parse(fs.readFileSync(path.join(dir, BASE_FILE), "utf8")) as Partial<Base>;
    if (saved.account === account && saved.skills && typeof saved.skills === "object") return { account, skills: saved.skills };
  } catch {}
  return { account, skills: {} };
}

/** Written whole and renamed into place, so a crash leaves the old base or the new one. */
function saveBase(dir: string, base: Base) {
  fs.mkdirSync(dir, { recursive: true });
  const temp = path.join(dir, `${BASE_FILE}.tmp`);
  fs.writeFileSync(temp, JSON.stringify(base, null, 2));
  fs.renameSync(temp, path.join(dir, BASE_FILE));
}

type Local = { hash: string; files: SkillFiles } | { unreadable: string };

/** This computer's copy of a skill, its hidden files left out as an import leaves them out; null without a SKILL.md. */
function readLocal(dir: string, name: string): Local | null {
  const folder = path.join(dir, name);
  if (!fs.existsSync(path.join(folder, SKILL_FILE))) return null;
  const files: SkillFiles = {};
  try {
    for (const entry of fs.readdirSync(folder, { recursive: true, withFileTypes: true })) {
      if (!entry.isFile()) continue;
      const file = path.relative(folder, path.join(entry.parentPath, entry.name)).split(path.sep).join("/");
      if (hiddenPart(file)) continue;
      try {
        files[file] = UTF8.decode(fs.readFileSync(path.join(folder, file)));
      } catch {
        return { unreadable: `${file} isn't a text file, so Linker can't keep it.` };
      }
    }
  } catch {
    return null;
  }
  return { hash: hashFiles(files), files };
}

function localNames(dir: string): string[] {
  try {
    return fs.readdirSync(dir).filter((name) => SKILL_NAME.test(name));
  } catch {
    return [];
  }
}

/** Why Linker didn't keep a skill; null when the refusal isn't about the skill but about the whole folder. */
function skipReason(error: unknown): string | null {
  if (!(error instanceof SkillsRefusal)) return null;
  if (error.code === "invalid_skill") return error.message === error.code ? "Linker can't keep it as it is." : error.message;
  if (error.code !== "too_large") return null;
  if (error.limit === "files") return `It has over ${SKILL_IMPORT_LIMITS.skillFiles} files, more than Linker keeps.`;
  if (error.limit === "bytes") return `It's over ${formatBytes(SKILL_IMPORT_LIMITS.skillBytes)}, more than Linker keeps.`;
  return "Linker keeps no more skills here. Remove some first.";
}

/** What's wrong with a whole folder, for its status line; doing is what was refused, in "can add its skills". */
function problemText(error: unknown, folder: SyncFolder, doing = "change"): string {
  const code = error instanceof SkillsRefusal ? error.code : "";
  if (code === "forbidden") return `Only admins and editors of ${folder.name} can ${doing} its skills`;
  if (code === "not_member") return `You're not in ${folder.name} on Linker any more`;
  if (code === "unavailable") return "Linker can't take skills yet";
  if (code === "offline") return "Couldn't reach Linker. Trying again in a minute";
  if (code === "unauthorized" || code === "signed_out") return "Linker didn't take your sign-in. Sign in again";
  return `Couldn't sync with Linker: ${error instanceof Error ? error.message : String(error)}`;
}

const SYNCING: SkillSync = { state: "local", text: "Syncing with Linker…", skipped: [] };

/** Over Linker's limits, said before sending it. */
function overLimits(files: SkillFiles): SkillsRefusal | null {
  if (Object.keys(files).length > SKILL_IMPORT_LIMITS.skillFiles) return new SkillsRefusal("too_large", undefined, "files");
  const bytes = Object.values(files).reduce((sum, text) => sum + Buffer.byteLength(text), 0);
  return bytes > SKILL_IMPORT_LIMITS.skillBytes ? new SkillsRefusal("too_large", undefined, "bytes") : null;
}

export interface SkillSyncOptions {
  api: SkillsApi;
  /** Linker's id for the person signed in; null when nobody is. */
  account(): string | null;
  /** The folders that sync right now. */
  folders(): SyncFolder[];
  /** Where this computer's copy of a skill goes when Linker's wins over it: outside every folder of skills. */
  bin: string;
  /** Skills on this computer, or how a folder stands with Linker, changed. */
  changed(): void;
  now?: () => Date;
}

export type SkillSyncService = ReturnType<typeof createSkillSync>;

export function createSkillSync(options: SkillSyncOptions) {
  const { api } = options;
  const now = options.now ?? (() => new Date());
  /** Linker's last list, for its etag; the account it was for. */
  let listed: { account: string; etag: string; skills: RemoteSkill[] } | null = null;
  const statuses = new Map<string, SkillSync>();
  /** Writes Linker refused, by folder and skill, for the files they were of: sent again only once those change. */
  const refused = new Map<string, { hash: string; skip: string | null; problem: string | null }>();
  const refusedKey = (dir: string, name: string) => `${dir}\0${name}`;

  // One pass, import or remove at a time: none reads a folder while another writes it.
  let lock: Promise<unknown> = Promise.resolve();
  function exclusive<T>(work: () => Promise<T>): Promise<T> {
    const result = lock.then(work);
    lock = result.catch(() => {});
    return result;
  }

  const folderOf = (dir: string) => (options.account() ? options.folders().find((folder) => folder.dir === dir) : undefined);

  function report(next: Map<string, SkillSync>, touched: boolean) {
    const before = JSON.stringify([...statuses]);
    statuses.clear();
    for (const [dir, status] of next) statuses.set(dir, status);
    if (touched || JSON.stringify([...statuses]) !== before) options.changed();
  }

  function bin(folder: SyncFolder, name: string) {
    const at = now().toISOString();
    const dest = path.join(options.bin, at.slice(0, 10), folder.owner, `${name}-${at.slice(11, 19).replaceAll(":", "-")}`);
    fs.cpSync(path.join(folder.dir, name), dest, { recursive: true });
    console.log(`The skill "${name}" changed both here and on Linker: Linker's is kept, and this computer's is in ${dest}`);
  }

  /** Brings one folder and Linker's skills of its owner together. Returns its status, and whether skills here changed. */
  async function reconcile(folder: SyncFolder, account: string, remote: RemoteSkill[]): Promise<{ status: SkillSync; touched: boolean }> {
    const { dir } = folder;
    const base = readBase(dir, account);
    const theirs = new Map(remote.map((skill) => [skill.name, skill]));
    const names = [...new Set([...localNames(dir), ...Object.keys(base.skills), ...theirs.keys()])].sort();
    const skipped: SkillSync["skipped"] = [];
    let problem: string | null = null;
    let touched = false;
    const agree = (name: string, agreed: Agreed | null) => {
      if (agreed) base.skills[name] = agreed;
      else delete base.skills[name];
      saveBase(dir, base);
    };
    const pull = (skill: RemoteSkill) => {
      const written = writeSkills(dir, [{ name: skill.name, description: "", files: new Map(Object.entries(skill.files)) }]);
      if (written.skipped.length) throw new Error(written.skipped[0]!.reason);
      agree(skill.name, { revision: skill.revision, hash: hashFiles(skill.files) });
      touched = true;
    };

    for (const name of names) {
      const local = readLocal(dir, name);
      if (local && "unreadable" in local) {
        skipped.push({ name, reason: local.unreadable });
        continue;
      }
      const skill = theirs.get(name) ?? null;
      const action = decide(local?.hash ?? null, base.skills[name], skill && { revision: skill.revision, hash: hashFiles(skill.files) });
      const key = refusedKey(dir, name);
      const known = refused.get(key);
      if ((action === "push" || action === "delete_remote") && known?.hash === (local?.hash ?? "")) {
        if (known.skip) skipped.push({ name, reason: known.skip });
        problem ??= known.problem;
        continue;
      }
      try {
        switch (action) {
          case "keep":
            break;
          case "adopt":
          case "forget":
            agree(name, skill && { revision: skill.revision, hash: hashFiles(skill.files) });
            break;
          case "remote_wins":
            bin(folder, name);
            pull(skill!);
            break;
          case "pull":
            pull(skill!);
            break;
          case "remove_local": {
            const refusal = removeSkill(dir, name);
            if (refusal) throw new Error(refusal);
            agree(name, null);
            touched = true;
            break;
          }
          case "push": {
            const files = (local as { files: SkillFiles; hash: string }).files;
            const tooLarge = overLimits(files);
            if (tooLarge) throw tooLarge;
            const kept = await api.put(folder.owner, name, files, skill?.revision ?? 0);
            agree(name, { revision: kept.revision, hash: local!.hash });
            break;
          }
          case "delete_remote":
            await api.remove(folder.owner, name).catch((error: unknown) => {
              if (!(error instanceof SkillsRefusal && error.code === "not_found")) throw error;
            });
            agree(name, null);
            break;
        }
        refused.delete(key);
      } catch (error) {
        // Linker's copy changed since the list: the next pass sees how.
        if (error instanceof SkillsRefusal && error.code === "conflict") continue;
        const skip = skipReason(error);
        if (skip) {
          skipped.push({ name, reason: skip });
          refused.set(key, { hash: local?.hash ?? "", skip, problem: null });
          continue;
        }
        problem = problemText(error, folder);
        // A viewer's change stays here, and doesn't stop what Linker sends.
        if (error instanceof SkillsRefusal && error.code === "forbidden") {
          refused.set(key, { hash: local?.hash ?? "", skip: null, problem });
          continue;
        }
        break;
      }
    }
    const synced = skipped.length ? `Synced with Linker, except ${skipped.length === 1 ? skipped[0]!.name : `${skipped.length} skills`}` : "Synced with Linker";
    return { status: { state: problem ? "problem" : "synced", text: problem ?? synced, skipped }, touched };
  }

  async function pass() {
    const account = options.account();
    if (!account) {
      listed = null;
      report(new Map(), false);
      return;
    }
    const folders = options.folders();
    try {
      if (listed?.account !== account) listed = null;
      const answer = await api.list(listed?.etag ?? null);
      if (answer) listed = { account, ...answer };
    } catch (error) {
      report(new Map(folders.map((folder) => [folder.dir, { state: "problem", text: problemText(error, folder), skipped: [] }])), false);
      return;
    }
    const next = new Map<string, SkillSync>();
    let touched = false;
    for (const folder of folders) {
      const done = await reconcile(
        folder,
        account,
        (listed?.skills ?? []).filter((skill) => skill.owner === folder.owner),
      );
      next.set(folder.dir, done.status);
      touched ||= done.touched;
    }
    report(next, touched);
  }

  let waiting: Promise<void> | null = null;
  /** A pass soon: one not started yet covers every change made before it starts. */
  function sync(): Promise<void> {
    waiting ??= exclusive(async () => {
      waiting = null;
      await pass();
    });
    return waiting;
  }

  return {
    sync,

    /** How a folder stands with Linker; null for one that doesn't sync right now. */
    status(dir: string): SkillSync | null {
      return folderOf(dir) ? (statuses.get(dir) ?? SYNCING) : null;
    },

    /**
     * Writes imported skills into dir; into a folder that syncs, through Linker first, so a person who may not add
     * to it is told so before anything is written. Unreachable, Linker gets them at a later pass.
     */
    write(dir: string, skills: ImportedSkill[]) {
      const result = exclusive(async () => {
        const folder = folderOf(dir);
        if (!folder) return writeSkills(dir, skills);
        const kept = new Map<string, number>();
        for (const skill of skills) {
          const files = Object.fromEntries(skill.files);
          try {
            kept.set(skill.name, (await api.put(folder.owner, skill.name, files, null)).revision);
          } catch (error) {
            const code = error instanceof SkillsRefusal ? error.code : "";
            if (code === "forbidden" || code === "not_member") return { error: `${problemText(error, folder, "add")}.` };
            const skip = skipReason(error);
            if (!skip) break;
            refused.set(refusedKey(dir, skill.name), { hash: hashFiles(files), skip, problem: null });
          }
        }
        const written = writeSkills(dir, skills);
        const base = readBase(dir, options.account()!);
        for (const skill of skills) {
          const revision = kept.get(skill.name);
          if (revision !== undefined && written.imported.includes(skill.name)) base.skills[skill.name] = { revision, hash: hashFiles(Object.fromEntries(skill.files)) };
        }
        if (kept.size) saveBase(dir, base);
        return written;
      });
      void result.catch(() => {}).then(() => sync());
      return result;
    },

    /** Removes a skill from dir; from a folder that syncs, from Linker first, so it doesn't come back. */
    remove(dir: string, name: string): Promise<string | null> {
      const result = exclusive(async () => {
        const folder = folderOf(dir);
        if (folder && SKILL_NAME.test(name)) {
          try {
            await api.remove(folder.owner, name);
          } catch (error) {
            const code = error instanceof SkillsRefusal ? error.code : "";
            if (code === "forbidden" || code === "not_member") return `${problemText(error, folder, "remove")}.`;
            // Unreachable: the base still has it, so a later pass removes it from Linker.
            if (code !== "not_found") return removeSkill(dir, name);
          }
          const base = readBase(dir, options.account()!);
          delete base.skills[name];
          saveBase(dir, base);
        }
        return removeSkill(dir, name);
      });
      void result.catch(() => {}).then(() => sync());
      return result;
    },
  };
}
