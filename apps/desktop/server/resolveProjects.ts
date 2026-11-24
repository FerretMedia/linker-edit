import type { LocalLink, ResolveDatabase, ResolveLibrary, ResolveLink } from "../shared/protocol.ts";
import { ResolveError, type ResolveConnection } from "./resolve/connection.ts";
import { startResolve, type StartOptions } from "./resolve/start.ts";

// The scripts are server/resolve/scripts/list_projects.py, open_project.py and create_project.py.

export interface OpenedProject {
  project: { name: string; id: string | null };
  database: ResolveDatabase | null;
  /** The Project Manager folders it was found in; null when that isn't known. */
  folder: string[] | null;
}

export interface ResolveProjects {
  library(): Promise<ResolveLibrary>;
  /** Starts Resolve when it isn't running, and waits until it answers. True when this started it. */
  start(signal?: AbortSignal): Promise<boolean>;
  /** Starts Resolve when it isn't running, saves the project it has open, then opens this one. */
  open(link: ResolveLink, local: LocalLink, options?: OpenOptions): Promise<OpenedProject>;
  /** Starts Resolve when it isn't running, saves the project it has open, then makes and opens a new one. */
  create(name: string): Promise<OpenedProject>;
}

export interface OpenOptions {
  /** Refuse when Resolve has another project open: for agents, which never switch projects. */
  onlyIfIdle?: boolean;
  signal?: AbortSignal;
}

/** How long opening may take: a big project can take minutes to load. */
const OPEN_TIMEOUT_MS = 5 * 60_000;

export function createResolveProjects(connection: ResolveConnection, pollMs = 3000, startOptions: StartOptions = {}): ResolveProjects {
  const start = (signal?: AbortSignal) => startResolve(connection, { ...startOptions, signal });
  return {
    start,
    library: () => connection.runScript<ResolveLibrary>("list_projects", {}, 60),
    async open(link, local, { onlyIfIdle = false, signal } = {}) {
      await start(signal);
      try {
        return await connection.runScript<OpenedProject>(
          "open_project",
          {
            kind: link.kind,
            name: link.name,
            projectId: link.projectId,
            database: local.database ?? null,
            folder: local.folder ?? [],
            onlyIfIdle,
          },
          60,
        );
      } catch (error) {
        // Scripts stop after 60 seconds, but Resolve goes on opening: wait for the project to show up.
        if (!(error instanceof ResolveError && (error.kind === "timeout" || error.kind === "unavailable"))) throw error;
        return waitForProject(connection, link, pollMs, signal);
      }
    },
    async create(name) {
      await start();
      return connection.runScript<OpenedProject>("create_project", { name }, 60);
    },
  };
}

async function waitForProject(connection: ResolveConnection, link: ResolveLink, pollMs: number, signal?: AbortSignal): Promise<OpenedProject> {
  const deadline = Date.now() + OPEN_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (signal?.aborted) throw new Error("Stopped while waiting for Resolve.");
    await new Promise((resolve) => setTimeout(resolve, pollMs));
    const status = await connection
      .runScript<{ project: string | null; projectId: string | null; database: ResolveDatabase | null }>("status", {}, 10)
      .catch(() => null);
    if (status?.project && (link.projectId ? status.projectId === link.projectId : status.project === link.name)) {
      return { project: { name: status.project, id: status.projectId }, database: status.database, folder: null };
    }
  }
  throw new Error(`Resolve didn't open "${link.name}" within 5 minutes. Is a dialog open in Resolve?`);
}
