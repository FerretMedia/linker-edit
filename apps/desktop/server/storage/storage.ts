// Settings > Computers and storage: this computer's disks and the space each project takes, and the last report
// each of the user's other computers gave.
import * as fs from "node:fs";
import * as path from "node:path";
import { Worker } from "node:worker_threads";
import type { ComputerStorage, StorageCard } from "../../shared/protocol.ts";
import { statOf, type FolderSize } from "./walk.ts";
import type { WalkAnswer, WalkRequest } from "./walkWorker.ts";

export interface StorageProject {
  projectId: string;
  name: string;
  dir: string;
}

export interface StorageOptions {
  /** Linker-Edit's own data folder, whose disk is always listed. */
  dataDir: string;
  /** Where the other computers' last reports are kept across restarts; null keeps them in memory only. */
  file: string | null;
  now?: () => Date;
  log?: (line: string) => void;
}

export type StorageService = ReturnType<typeof createStorage>;

// In the npm package this module is bundled into dist/, next to the walk's own bundle.
const BUNDLED_WORKER = path.join(import.meta.dirname, "storage-walk.js");
const workerFile = () => (fs.existsSync(BUNDLED_WORKER) ? BUNDLED_WORKER : path.join(import.meta.dirname, "walkWorker.ts"));

/** The disk a folder is on, as the topmost folder above it on the same device; null when it isn't there. */
function mountOf(dir: string): string | null {
  let current = path.resolve(dir);
  const device = statOf(current)?.dev;
  if (device === undefined) return null;
  for (let up = path.dirname(current); up !== current && statOf(up)?.dev === device; up = path.dirname(current)) current = up;
  return current;
}

/** "C:" for C:\, "Media" for /Volumes/Media, "/" for /. */
const labelOf = (mount: string) => path.basename(mount) || mount.replace(/[\\/]+$/, "") || mount;

async function disksOf(dirs: string[]): Promise<ComputerStorage["disks"]> {
  const mounts = [...new Set(dirs.map(mountOf).filter((mount) => mount !== null))].sort();
  const disks = await Promise.all(
    mounts.map(async (mount) => {
      try {
        const stats = await fs.promises.statfs(mount);
        return { path: mount, label: labelOf(mount), totalBytes: stats.blocks * stats.bsize, freeBytes: stats.bavail * stats.bsize };
      } catch {
        return null;
      }
    }),
  );
  return disks.filter((disk) => disk !== null);
}

const isNumber = (value: unknown) => typeof value === "number" && Number.isFinite(value);
const isNullable = (value: unknown, check: (value: unknown) => boolean) => value === null || check(value);
const MIRROR_STATES = new Set<unknown>(["cloud", "downloading", "mirrored", "error"]);

/** Another computer's answer, checked before it's kept and shown. */
export function isComputerStorage(value: unknown): value is ComputerStorage {
  const report = value as Partial<Record<keyof ComputerStorage, unknown>> | null;
  return (
    !!report &&
    typeof report.computerId === "string" &&
    typeof report.measuredAt === "string" &&
    !Number.isNaN(Date.parse(report.measuredAt)) &&
    Array.isArray(report.disks) &&
    report.disks.every((disk: Record<string, unknown>) =>
      typeof disk?.path === "string" && typeof disk.label === "string" && isNumber(disk.totalBytes) && isNumber(disk.freeBytes),
    ) &&
    Array.isArray(report.projects) &&
    report.projects.every(
      (project: Record<string, unknown>) =>
        typeof project?.projectId === "string" &&
        typeof project.name === "string" &&
        isNullable(project.localBytes, isNumber) &&
        isNullable(project.cloudBytes, isNumber) &&
        isNullable(project.mirror, (mirror) => MIRROR_STATES.has(mirror)),
    )
  );
}

function readReports(file: string | null): Map<string, ComputerStorage> {
  if (!file) return new Map();
  try {
    const saved = (JSON.parse(fs.readFileSync(file, "utf8")) as { reports?: unknown }).reports;
    return new Map(Object.entries(saved && typeof saved === "object" ? saved : {}).filter((entry): entry is [string, ComputerStorage] => isComputerStorage(entry[1])));
  } catch {
    return new Map();
  }
}

export function createStorage(options: StorageOptions) {
  const now = options.now ?? (() => new Date());
  const log = options.log ?? console.log;

  let worker: Worker | null = null;
  let lastId = 0;
  const waiting = new Map<number, { resolve: (sizes: (FolderSize | null)[]) => void; reject: (error: Error) => void }>();

  /** Each folder's size, walked on the worker thread, which keeps their listings for the next walk. */
  function walk(roots: string[]): Promise<(FolderSize | null)[]> {
    if (!worker) {
      const started = new Worker(workerFile());
      started.unref();
      started.on("message", ({ id, sizes }: WalkAnswer) => {
        waiting.get(id)?.resolve(sizes);
        waiting.delete(id);
      });
      const fail = (error: Error) => {
        if (worker === started) worker = null;
        for (const entry of waiting.values()) entry.reject(error);
        waiting.clear();
      };
      started.on("error", fail);
      started.on("exit", (code) => fail(new Error(`The folder walk stopped (exit code ${code}).`)));
      worker = started;
    }
    const id = ++lastId;
    const running = worker;
    return new Promise((resolve, reject) => {
      waiting.set(id, { resolve, reject });
      running.postMessage({ id, roots } satisfies WalkRequest);
    });
  }

  async function measureNow(computerId: string, projects: StorageProject[]): Promise<ComputerStorage> {
    const started = performance.now();
    const sizes = await walk(projects.map((project) => project.dir));
    const disks = await disksOf([options.dataDir, ...projects.map((project) => project.dir)]);
    const rows = projects
      .map((project, index) => ({ projectId: project.projectId, name: project.name, localBytes: sizes[index]?.bytes ?? null, cloudBytes: null, mirror: null }))
      .sort((a, b) => (b.localBytes ?? -1) - (a.localBytes ?? -1));
    const total: FolderSize = { bytes: 0, files: 0, listed: 0, reused: 0 };
    for (const size of sizes) for (const key of Object.keys(total) as (keyof FolderSize)[]) total[key] += size?.[key] ?? 0;
    log(
      `Storage: ${projects.length} projects, ${total.files} files, ${total.bytes} bytes in ${Math.round(performance.now() - started)} ms (${total.listed} folders read, ${total.reused} unchanged)`,
    );
    return { computerId, disks, projects: rows, measuredAt: now().toISOString() };
  }

  let measuring: Promise<ComputerStorage> | null = null;
  /** The last report each computer gave, by machine id. */
  const reports = readReports(options.file);

  function saveReports() {
    if (!options.file) return;
    try {
      fs.mkdirSync(path.dirname(options.file), { recursive: true });
      const temp = `${options.file}.tmp`;
      fs.writeFileSync(temp, `${JSON.stringify({ reports: Object.fromEntries(reports) })}\n`);
      fs.renameSync(temp, options.file);
    } catch (error) {
      console.error(error);
    }
  }

  return {
    /** This computer's report; asked again while one is measured, it waits for that one. */
    measure(computerId: string, projects: StorageProject[]): Promise<ComputerStorage> {
      measuring ??= measureNow(computerId, projects).finally(() => (measuring = null));
      return measuring;
    },
    /** A computer's report as ask gets it; when that fails, the last one it gave, marked stale. */
    async card(machineId: string, ask: () => Promise<unknown>): Promise<StorageCard> {
      try {
        const report = await ask();
        if (!isComputerStorage(report)) throw new Error("It answered with something that isn't a storage report.");
        reports.set(machineId, report);
        saveReports();
        return { state: "fresh", report };
      } catch (error) {
        const problem = error instanceof Error ? error.message : String(error);
        const last = reports.get(machineId);
        return last ? { state: "stale", report: last, problem } : { state: "none", problem };
      }
    },
  };
}
