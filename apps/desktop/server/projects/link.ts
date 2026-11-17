import type { LocalLink, ResolveDatabase, ResolveLink, ResolveStatus } from "../../shared/protocol.ts";

/** The project open in Resolve, as the status probe reports it. */
export interface OpenProject {
  name: string;
  id: string | null;
  database: ResolveDatabase | null;
}

export function openProject(status: ResolveStatus): OpenProject | null {
  if (status.state !== "connected" || !status.project) return null;
  return { name: status.project, id: status.projectId, database: status.database };
}

// Resolve's scripting API documents only "Disk" and "PostgreSQL"; what it reports for a Blackmagic
// Cloud library is unverified, so any other type counts as cloud.
export function linkKind(database: ResolveDatabase | null): ResolveLink["kind"] {
  if (!database || database.type.toLowerCase() === "disk") return "local";
  return database.type.toLowerCase() === "postgresql" ? "network" : "cloud";
}

const sameDatabase = (a: ResolveDatabase, b: ResolveDatabase) =>
  a.type.toLowerCase() === b.type.toLowerCase() && a.name === b.name && (a.host ?? "") === (b.host ?? "");

/**
 * Is the open project this link's? By Resolve's project id once both sides know it, so a rename in
 * Resolve still matches; by name and library before that.
 */
export function isLinkedProject(link: ResolveLink, local: LocalLink, open: OpenProject): boolean {
  if (link.projectId && open.id) return link.projectId === open.id;
  if (link.name !== open.name || link.kind !== linkKind(open.database)) return false;
  return !local.database || !open.database || sameDatabase(local.database, open.database);
}

export function linkFromOpen(open: OpenProject): { link: ResolveLink; local: LocalLink } {
  return {
    link: { kind: linkKind(open.database), name: open.name, projectId: open.id },
    local: open.database ? { database: open.database } : {},
  };
}

/** The link brought up to date with what Resolve reports for the same project: its id, and a rename. */
export function refreshedLink(link: ResolveLink, local: LocalLink, open: OpenProject): { link: ResolveLink; local: LocalLink } {
  return {
    link: { ...link, name: open.name, projectId: open.id ?? link.projectId },
    local: open.database ? { ...local, database: open.database } : local,
  };
}
