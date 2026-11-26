import { createHash } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import { DriveError, FOLDER_TYPE, type DriveChange, type DriveClient, type DriveFile } from "../client.ts";
import { applyChanges, excluded, planSync, remotePaths, type Action, type BaseFile, type LocalFile, type RemoteTree } from "./plan.ts";

/**
 * One pass of the Drive sync for one project folder: look at both sides, plan, and carry the plan out one step at a
 * time. Each step writes the manifest, so a pass killed anywhere carries on from where it stopped: the next pass
 * plans again from what is there.
 */

export const MANIFEST_FILE = ".linker-edit/drive-manifest.json";
/** Downloads arrive here, named by Drive id and md5, until their md5 matches. */
const DOWNLOADS = ".linker-edit/drive-downloads";

interface PendingUpload {
  /** The resumable session's address; its own credential. */
  session: string;
  md5: string;
  size: number;
}

export interface SyncManifest {
  version: 1;
  /** Where Drive's changes feed carries on from: remote has everything before it. */
  pageToken: string;
  remote: RemoteTree;
  /** What this computer and Drive last agreed on, by path. */
  files: Record<string, BaseFile>;
  /** Uploads under way, by path, so a restart carries on with the same session. */
  uploads: Record<string, PendingUpload>;
}

export interface SyncOptions {
  drive: DriveClient;
  dir: string;
  /** The project's folder on Drive. */
  folderId: string;
  /** This computer's name, for conflict copies. */
  computer: string;
  /** Bytes moved so far out of what this pass moves. */
  onProgress?: (done: number, total: number) => void;
  log?: (line: string) => void;
}

export interface SyncResult {
  /** Paths on this computer the pass wrote, moved or deleted. */
  changed: string[];
  failures: { path: string; message: string }[];
  /** A file changed while the pass worked on it, so it was left for the next one. */
  skipped: string[];
}

/** Why a pass can't start, worded for the user. */
export class SyncStopped extends Error {}

const dirOf = (file: string) => {
  const dir = path.posix.dirname(file);
  return dir === "." ? "" : dir;
};
const quote = (name: string) => name.replace(/\\/g, "\\\\").replace(/'/g, "\\'");
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

async function md5File(file: string): Promise<string> {
  const hash = createHash("md5");
  for await (const chunk of fs.createReadStream(file)) hash.update(chunk as Buffer);
  return hash.digest("hex");
}

function readManifest(file: string): SyncManifest | null {
  try {
    const saved = JSON.parse(fs.readFileSync(file, "utf8")) as Partial<SyncManifest> | null;
    const ok = saved?.version === 1 && typeof saved.pageToken === "string" && !!saved.remote?.files && !!saved.remote.folders && !!saved.files && !!saved.uploads;
    return ok ? (saved as SyncManifest) : null;
  } catch {
    // Missing or unreadable: the pass starts again from a full look at Drive, which never deletes anything.
    return null;
  }
}

function writeAtomically(file: string, value: unknown) {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  const temp = `${file}.tmp`;
  fs.writeFileSync(temp, JSON.stringify(value));
  fs.renameSync(temp, file);
}

export async function syncProject({ drive, dir, folderId, computer, onProgress, log = () => {} }: SyncOptions): Promise<SyncResult> {
  const manifestFile = path.join(dir, MANIFEST_FILE);
  const abs = (file: string) => path.join(dir, ...file.split("/"));
  const result: SyncResult = { changed: [], failures: [], skipped: [] };

  const root = await drive.get(folderId).catch((error: unknown) => {
    if (error instanceof DriveError && error.status === 404) throw new SyncStopped("The project's folder on Google Drive is gone, or this account can't see it.");
    throw error;
  });
  if (root.trashed) throw new SyncStopped("The project's folder is in Google Drive's bin. Restore it there to sync again.");

  let manifest = readManifest(manifestFile) ?? (await firstLook());
  const save = () => writeAtomically(manifestFile, manifest);

  async function firstLook(): Promise<SyncManifest> {
    // Taken first: whatever changes while the folders are listed comes in through the feed.
    const pageToken = await drive.startPageToken();
    const found: DriveChange[] = [];
    for (const folders = [folderId]; folders.length > 0; ) {
      for (const file of await drive.list(`'${folders.pop()}' in parents and trashed = false`)) {
        found.push({ fileId: file.id, removed: false, file });
        if (file.mimeType === FOLDER_TYPE) folders.push(file.id);
      }
    }
    const first: SyncManifest = { version: 1, pageToken, remote: applyChanges({ folders: {}, files: {} }, found, folderId), files: {}, uploads: {} };
    writeAtomically(manifestFile, first);
    return first;
  }

  const feed = await drive.changes(manifest.pageToken);
  if (feed.changes.length > 0 || feed.pageToken !== manifest.pageToken) {
    manifest = { ...manifest, pageToken: feed.pageToken, remote: applyChanges(manifest.remote, feed.changes, folderId) };
    save();
  }

  const local = await scan(manifest.files);
  const view = remotePaths(manifest.remote, folderId);
  const actions = planSync({ base: manifest.files, local, remote: view.files, computer });

  const uploading = new Set(actions.flatMap((action) => (action.kind === "upload" ? [action.path] : [])));
  if (Object.keys(manifest.uploads).some((file) => !uploading.has(file))) {
    manifest.uploads = Object.fromEntries(Object.entries(manifest.uploads).filter(([file]) => uploading.has(file)));
    save();
  }
  clearOldDownloads(actions);
  if (actions.length === 0) return result;

  // A conflict copy is only on disk once its setAside ran.
  const copied = new Map(actions.flatMap((action) => (action.kind === "setAside" ? [[action.copy, action.path] as const] : [])));
  const bytes = new Map(
    actions.map((action) => [
      action,
      action.kind === "upload" ? local[copied.get(action.path) ?? action.path]!.size : action.kind === "download" ? manifest.remote.files[action.driveId]!.size : 0,
    ]),
  );
  const total = [...bytes.values()].reduce((sum, size) => sum + size, 0);
  let done = 0;
  onProgress?.(0, total);
  const started = Date.now();
  for (const action of actions) {
    const at = "path" in action ? action.path : action.from;
    try {
      await run(action, (moved) => onProgress?.(done + moved, total));
    } catch (error) {
      result.failures.push({ path: at, message: errorText(error) });
      log(`couldn't sync ${at}: ${errorText(error)}`);
    }
    done += bytes.get(action)!;
    onProgress?.(done, total);
  }
  log(`${actions.length} ${actions.length === 1 ? "change" : "changes"} in ${((Date.now() - started) / 1000).toFixed(1)} s`);
  return result;

  /** Every file in the project folder, with its md5: worked out again only when its size or time changed. */
  async function scan(base: Record<string, BaseFile>): Promise<Record<string, LocalFile>> {
    const found: Record<string, LocalFile> = {};
    // What can't be read now is taken as unchanged: an unreadable folder must never look like a deleted one.
    const unchanged = (prefix: string) => {
      for (const [file, entry] of Object.entries(base)) if (file.startsWith(prefix)) found[file] = { md5: entry.md5, size: entry.size, mtimeMs: entry.mtimeMs };
    };
    async function walk(rel: string) {
      let entries: fs.Dirent[];
      try {
        entries = await fs.promises.readdir(rel ? abs(rel) : dir, { withFileTypes: true });
      } catch (error) {
        if (!rel) throw new SyncStopped(`Can't read the project folder ${dir}: ${errorText(error)}`);
        return unchanged(`${rel}/`);
      }
      for (const entry of entries) {
        const file = rel ? `${rel}/${entry.name}` : entry.name;
        if (entry.isDirectory()) {
          if (!excluded(`${file}/`)) await walk(file);
        } else if (entry.isFile() && !excluded(file)) {
          const known = base[file];
          try {
            const stat = await fs.promises.stat(abs(file));
            const md5 = known && known.size === stat.size && known.mtimeMs === stat.mtimeMs ? known.md5 : await md5File(abs(file));
            found[file] = { md5, size: stat.size, mtimeMs: stat.mtimeMs };
          } catch {
            if (known) found[file] = { md5: known.md5, size: known.size, mtimeMs: known.mtimeMs };
          }
        }
      }
    }
    await walk("");
    return found;
  }

  /** Whether a file is still as the scan found it, or still missing. */
  async function unchangedSinceScan(file: string): Promise<boolean> {
    const was = local[file];
    const now = await fs.promises.stat(abs(file)).catch(() => null);
    return was ? !!now?.isFile() && now.size === was.size && now.mtimeMs === was.mtimeMs : !now;
  }

  /** Drive's file is still as the remote tree has it. */
  async function unchangedOnDrive(driveId: string): Promise<boolean> {
    const known = manifest.remote.files[driveId];
    const now = await drive.get(driveId);
    return !now.trashed && now.md5 === known?.md5 && now.name === known.name && now.parents[0] === known.parent;
  }

  function skip(file: string) {
    result.skipped.push(file);
    log(`left ${file} for next time: it changed while syncing`);
  }

  /** The Drive folder at a path, found by name before it's made: a retried create can leave two. */
  async function folder(at: string): Promise<string> {
    const known = view.folders[at];
    if (known) return known;
    const parent = await folder(dirOf(at));
    const name = path.posix.basename(at);
    const same = await drive.list(`'${parent}' in parents and name = '${quote(name)}' and mimeType = '${FOLDER_TYPE}' and trashed = false`);
    // Every computer takes the lowest id, as remotePaths does.
    const found: DriveFile = same.sort((a, b) => (a.id < b.id ? -1 : 1))[0] ?? (await drive.createFolder(name, parent));
    manifest.remote.folders[found.id] = { name, parent };
    view.folders[at] = found.id;
    save();
    return found.id;
  }

  function clearOldDownloads(planned: Action[]) {
    const wanted = new Set(planned.flatMap((action) => (action.kind === "download" ? [`${action.driveId}-${manifest.remote.files[action.driveId]!.md5}.part`] : [])));
    for (const name of fs.existsSync(path.join(dir, DOWNLOADS)) ? fs.readdirSync(path.join(dir, DOWNLOADS)) : []) {
      if (!wanted.has(name)) fs.rmSync(path.join(dir, DOWNLOADS, name), { force: true });
    }
  }

  function logSpeed(verb: string, file: string, bytes: number, since: number) {
    const seconds = Math.max((Date.now() - since) / 1000, 0.001);
    log(`${verb} ${file}: ${bytes} bytes in ${seconds.toFixed(1)} s, ${(bytes / seconds / 1e6).toFixed(1)} MB/s`);
  }

  async function run(action: Action, progress: (moved: number) => void): Promise<void> {
    switch (action.kind) {
      case "upload": {
        const file = action.path;
        const was = local[file];
        if (!was || !(await unchangedSinceScan(file))) return skip(file);
        if (action.driveId && !(await unchangedOnDrive(action.driveId))) return skip(file);
        const parent = await folder(dirOf(file));
        const since = Date.now();
        const pending = manifest.uploads[file];
        let sent: DriveFile | null = null;
        if (pending?.md5 === was.md5 && pending.size === was.size) {
          sent = await drive.upload(pending.session, abs(file), { resume: true, onProgress: progress }).catch((error: unknown) => {
            // The session ran out: start another.
            if (error instanceof DriveError && (error.status === 404 || error.status === 410)) return null;
            throw error;
          });
        }
        if (!sent) {
          const session = await drive.startUpload(action.driveId ? { fileId: action.driveId } : { name: path.posix.basename(file), parent }, was.size);
          manifest.uploads[file] = { session, md5: was.md5, size: was.size };
          save();
          sent = await drive.upload(session, abs(file), { onProgress: progress });
        }
        delete manifest.uploads[file];
        manifest.remote.files[sent.id] = { name: sent.name, parent: sent.parents[0] ?? parent, md5: sent.md5!, size: sent.size! };
        // Drive has other bytes than the scan hashed when the file changed during the upload: hash it again next time.
        manifest.files[file] = { driveId: sent.id, md5: sent.md5!, size: sent.size!, mtimeMs: sent.md5 === was.md5 ? was.mtimeMs : -1 };
        save();
        return logSpeed("uploaded", file, was.size, since);
      }
      case "download": {
        const file = action.path;
        const want = manifest.remote.files[action.driveId]!;
        if (!(await unchangedSinceScan(file))) return skip(file);
        const part = path.join(dir, DOWNLOADS, `${action.driveId}-${want.md5}.part`);
        fs.mkdirSync(path.dirname(part), { recursive: true });
        const from = fs.existsSync(part) ? Math.min(fs.statSync(part).size, want.size) : 0;
        const since = Date.now();
        if (from === 0 || from < want.size) await drive.download(action.driveId, part, from, progress);
        if ((await md5File(part)) !== want.md5) {
          fs.rmSync(part, { force: true });
          throw new Error("It didn't arrive whole from Google Drive. It downloads again next time.");
        }
        // Changed while it downloaded: the download waits in DOWNLOADS for the next pass.
        if (!(await unchangedSinceScan(file))) return skip(file);
        fs.mkdirSync(path.dirname(abs(file)), { recursive: true });
        fs.renameSync(part, abs(file));
        manifest.files[file] = { driveId: action.driveId, md5: want.md5, size: want.size, mtimeMs: fs.statSync(abs(file)).mtimeMs };
        save();
        result.changed.push(file);
        return logSpeed("downloaded", file, want.size - from, since);
      }
      case "moveLocal": {
        if (!(await unchangedSinceScan(action.from)) || !(await unchangedSinceScan(action.to))) return skip(action.from);
        fs.mkdirSync(path.dirname(abs(action.to)), { recursive: true });
        fs.renameSync(abs(action.from), abs(action.to));
        manifest.files[action.to] = { ...manifest.files[action.from]!, mtimeMs: fs.statSync(abs(action.to)).mtimeMs };
        delete manifest.files[action.from];
        save();
        result.changed.push(action.from, action.to);
        return;
      }
      case "moveRemote": {
        if (!(await unchangedSinceScan(action.to)) || !(await unchangedOnDrive(action.driveId))) return skip(action.to);
        const was = manifest.remote.files[action.driveId]!;
        const parent = await folder(dirOf(action.to));
        const moved = await drive.move(action.driveId, path.posix.basename(action.to), was.parent, parent);
        manifest.remote.files[action.driveId] = { ...was, name: moved.name, parent };
        manifest.files[action.to] = { ...manifest.files[action.from]!, mtimeMs: local[action.to]!.mtimeMs };
        delete manifest.files[action.from];
        save();
        return;
      }
      case "deleteLocal": {
        if (!(await unchangedSinceScan(action.path))) return skip(action.path);
        fs.rmSync(abs(action.path), { force: true });
        delete manifest.files[action.path];
        save();
        result.changed.push(action.path);
        return;
      }
      case "deleteRemote": {
        if (!(await unchangedOnDrive(action.driveId))) return skip(action.path);
        await drive.trash(action.driveId);
        delete manifest.remote.files[action.driveId];
        delete manifest.files[action.path];
        save();
        return;
      }
      case "setAside": {
        if (!(await unchangedSinceScan(action.path)) || !(await unchangedSinceScan(action.copy))) return skip(action.path);
        fs.renameSync(abs(action.path), abs(action.copy));
        // The copy uploads from its new name, as the scan found it under the old one.
        local[action.copy] = local[action.path]!;
        delete local[action.path];
        result.changed.push(action.path, action.copy);
        return;
      }
      case "agree":
        manifest.files[action.path] = action.file;
        return save();
      case "forget":
        delete manifest.files[action.path];
        return save();
    }
  }
}
