import type { LocalLink, ResolveDatabase, ResolveLibrary, ResolveLink } from "../shared/protocol.ts";
import { ResolveError, type ResolveConnection } from "./resolve/connection.ts";

// The scripts are server/resolve/scripts/list_projects.py and open_project.py.

export interface OpenedProject {
  project: { name: string; id: string | null };
  database: ResolveDatabase | null;
  /** The Project Manager folders it was found in; null when that isn't known. */
  folder: string[] | null;
}

export interface ResolveProjects {
  library(): Promise<ResolveLibrary>;
  /** Saves Resolve's open project, then opens this one. */
  open(link: ResolveLink, local: LocalLink): Promise<OpenedProject>;
}

/** How long opening may take: a cloud project can download first. */
const OPEN_TIMEOUT_MS = 5 * 60_000;

export function createResolveProjects(connection: ResolveConnection, pollMs = 3000): ResolveProjects {
  return {
    library: () => connection.runScript<ResolveLibrary>("list_projects", {}, 60),
    async open(link, local) {
      try {
        return await connection.runScript<OpenedProject>(
          "open_project",
          {
            kind: link.kind,
            name: link.name,
            projectId: link.projectId,
            database: local.database ?? null,
            folder: local.folder ?? [],
            mediaPath: local.mediaPath ?? null,
            syncMode: local.syncMode ?? null,
          },
          60,
        );
      } catch (error) {
        // Scripts stop after 60 seconds, but Resolve goes on opening: wait for the project to show up.
        if (!(error instanceof ResolveError && (error.kind === "timeout" || error.kind === "unavailable"))) throw error;
        return waitForProject(connection, link, pollMs);
      }
    },
  };
}

async function waitForProject(connection: ResolveConnection, link: ResolveLink, pollMs: number): Promise<OpenedProject> {
  const deadline = Date.now() + OPEN_TIMEOUT_MS;
  while (Date.now() < deadline) {
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
