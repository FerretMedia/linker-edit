import type { ClientMessage, CollectionIcon, CollectionKind, CollectionMessage, CollectionSummary, IconColor } from "../../shared/protocol.ts";
import { COLLECTION_LABEL } from "../../src/lib/labels.ts";
import type { ProjectCollection, SavedProject } from "../projects/store.ts";
import { agentRules, PROJECT_NOTES_FILE, readProjectNotes, type RuleSource } from "../rules.ts";
import type { CollectionStore, SavedCollection } from "./store.ts";

/** What the workspaces and series need from the app around them. */
export interface CollectionsHost {
  store: CollectionStore;
  projects(): SavedProject[];
  /** Saves the project in its new workspace or series, or in none; commitMessage is for its edit history. */
  setCollection(projectId: string, collection: ProjectCollection | null, commitMessage: string): void;
  /** Clients and the agents' rules need to see a change. */
  changed(): void;
}

export const ICON_COLORS: readonly IconColor[] = ["gray", "red", "orange", "amber", "green", "teal", "blue", "violet", "pink"];

const KINDS: readonly CollectionKind[] = ["workspace", "series"];
const MAX_NAME = 80;
// RULES.md is for a style guide, not a book; this keeps a paste accident out of every prompt.
const MAX_RULES = 20_000;

const isMessage = (message: ClientMessage): message is CollectionMessage =>
  ["create_collection", "update_collection", "delete_collection", "move_project"].includes(message.type);

export function isCollectionIcon(value: unknown): value is CollectionIcon {
  if (!value || typeof value !== "object") return false;
  const icon = value as Record<string, unknown>;
  if (!ICON_COLORS.includes(icon.color as IconColor)) return false;
  switch (icon.kind) {
    case "lucide":
      return typeof icon.name === "string" && /^[a-z0-9-]{1,40}$/.test(icon.name);
    case "emoji":
      return typeof icon.emoji === "string" && icon.emoji.length > 0 && icon.emoji.length <= 16;
    case "monogram":
      return typeof icon.text === "string" && icon.text.length <= 3;
    default:
      return false;
  }
}

/** A new collection gets letters from its name, on a color picked by the name. */
function defaultIcon(name: string): CollectionIcon {
  let hash = 0;
  for (const char of name) hash = (hash * 31 + char.charCodeAt(0)) >>> 0;
  return { kind: "monogram", text: "", color: ICON_COLORS[1 + (hash % (ICON_COLORS.length - 1))]! };
}

/** The workspace a project is in, directly or through its series, and the series. */
export function collectionChain(
  collections: Map<string, SavedCollection>,
  project: SavedProject,
): { workspace: SavedCollection | null; series: SavedCollection | null } {
  const own = project.collection ? collections.get(project.collection.id) : undefined;
  if (!own) return { workspace: null, series: null };
  if (own.kind === "workspace") return { workspace: own, series: null };
  return { workspace: (own.parentId ? collections.get(own.parentId) : undefined) ?? null, series: own };
}

/**
 * Workspaces and series: groups above projects, each with its own rules for the agent. A workspace holds series
 * and projects; a series holds projects. One level deep: a series can't hold a series.
 */
export function createCollections(host: CollectionsHost) {
  const { store } = host;
  const collections = new Map<string, SavedCollection>();
  for (const collection of store.load()) {
    if (KINDS.includes(collection.kind)) collections.set(collection.id, collection);
  }
  // A series whose workspace went missing stands on its own rather than disappearing.
  for (const collection of collections.values()) {
    const parent = collection.parentId ? collections.get(collection.parentId) : undefined;
    if (collection.parentId && (collection.kind !== "series" || parent?.kind !== "workspace")) collection.parentId = null;
  }

  const label = (collection: SavedCollection) => `${COLLECTION_LABEL[collection.kind].one} "${collection.name}"`;

  /** Why parentId can't hold a collection of this kind; null when it can. */
  function parentRefusal(kind: CollectionKind, parentId: string | null): string | null {
    if (parentId === null) return null;
    if (kind === "workspace") return `A ${COLLECTION_LABEL.workspace.lower} can't go inside another one.`;
    const parent = collections.get(parentId);
    if (!parent) return `That ${COLLECTION_LABEL.workspace.lower} is gone.`;
    if (parent.kind !== "workspace") return `A ${COLLECTION_LABEL.series.lower} can only go in a ${COLLECTION_LABEL.workspace.lower}.`;
    return null;
  }

  function cleanName(name: unknown): string | null {
    if (typeof name !== "string") return null;
    const cleaned = name.replace(/\s+/g, " ").trim().slice(0, MAX_NAME);
    return cleaned || null;
  }

  /** Projects keep the name of their collection in linker-edit.json; a rename updates it there. */
  function renameInProjects(collection: SavedCollection) {
    for (const project of host.projects()) {
      if (project.collection?.id === collection.id && project.collection.name !== collection.name) {
        host.setCollection(project.id, { id: collection.id, name: collection.name }, `${label(collection)} renamed`);
      }
    }
  }

  function create(message: Extract<CollectionMessage, { type: "create_collection" }>): string | null {
    if (!KINDS.includes(message.kind)) return "Pick a workspace or a series.";
    const name = cleanName(message.name);
    if (!name) return "Give it a name.";
    const parentId = message.kind === "series" ? (message.parentId ?? null) : null;
    const refusal = parentRefusal(message.kind, parentId);
    if (refusal) return refusal;
    const icon = isCollectionIcon(message.icon) ? message.icon : defaultIcon(name);
    const collection = store.create({ kind: message.kind, name, parentId, icon, rules: "" });
    collections.set(collection.id, collection);
    return null;
  }

  function update(message: Extract<CollectionMessage, { type: "update_collection" }>): string | null {
    const current = collections.get(message.id);
    if (!current) return null;
    const next = { ...current };
    if (message.name !== undefined) {
      const name = cleanName(message.name);
      if (!name) return "Give it a name.";
      next.name = name;
    }
    if (message.icon !== undefined) {
      if (!isCollectionIcon(message.icon)) return null;
      next.icon = message.icon;
    }
    if (message.rules !== undefined) {
      if (typeof message.rules !== "string") return null;
      if (message.rules.length > MAX_RULES) return `Rules can be up to ${MAX_RULES.toLocaleString("en-US")} characters.`;
      next.rules = message.rules;
    }
    if (message.parentId !== undefined) {
      const refusal = parentRefusal(next.kind, message.parentId);
      if (refusal) return refusal;
      next.parentId = message.parentId;
    }
    collections.set(next.id, next);
    store.save(next);
    if (next.name !== current.name) renameInProjects(next);
    return null;
  }

  function remove(id: string): string | null {
    const collection = collections.get(id);
    if (!collection) return null;
    const series = [...collections.values()].filter((other) => other.parentId === id).length;
    const projects = host.projects().filter((project) => project.collection?.id === id).length;
    if (series || projects) {
      const what = [
        series ? `${series} ${series === 1 ? COLLECTION_LABEL.series.lower : COLLECTION_LABEL.series.lowerMany}` : "",
        projects ? `${projects} project${projects === 1 ? "" : "s"}` : "",
      ]
        .filter(Boolean)
        .join(" and ");
      return `${label(collection)} still has ${what} in it. Move ${series + projects === 1 ? "it" : "them"} out first.`;
    }
    collections.delete(id);
    store.remove(id);
    return null;
  }

  function move(projectId: string, collectionId: string | null): string | null {
    const project = host.projects().find((candidate) => candidate.id === projectId);
    if (!project) return null;
    if (collectionId === null) {
      if (project.collection) host.setCollection(projectId, null, `Take the project out of "${project.collection.name}"`);
      return null;
    }
    const collection = collections.get(collectionId);
    if (!collection) return `That ${COLLECTION_LABEL.workspace.lower} or ${COLLECTION_LABEL.series.lower} is gone.`;
    if (project.collection?.id !== collection.id) {
      host.setCollection(projectId, { id: collection.id, name: collection.name }, `Put the project in ${label(collection)}`);
    }
    return null;
  }

  return {
    /** Workspaces first, then series, each in the order they were made. */
    summaries(): CollectionSummary[] {
      const projects = host.projects();
      return [...collections.values()]
        .sort((a, b) => KINDS.indexOf(a.kind) - KINDS.indexOf(b.kind))
        .map((collection) => ({
          id: collection.id,
          kind: collection.kind,
          name: collection.name,
          parentId: collection.parentId,
          icon: collection.icon,
          rules: collection.rules,
          rulesFile: store.rulesFile(collection.id) ?? "",
          projectIds: projects.filter((project) => project.collection?.id === collection.id).map((project) => project.id),
        }));
    },

    /** The agent's rules for this project: the editing rules, then its workspace's, its series' and its own. */
    rulesFor(project: SavedProject): string {
      const { workspace, series } = collectionChain(collections, project);
      const sources: RuleSource[] = [];
      if (workspace) sources.push({ heading: `${label(workspace)} (its RULES.md)`, text: workspace.rules });
      if (series) sources.push({ heading: `${label(series)} (its RULES.md)`, text: series.rules });
      sources.push({ heading: `This project's notes (${PROJECT_NOTES_FILE} in the working folder)`, text: readProjectNotes(project.dir) });
      return agentRules(sources);
    },

    /** Handles the workspace and series messages; others are left alone. Returns why it was refused, if it was. */
    handle(message: ClientMessage): string | null {
      if (!isMessage(message)) return null;
      let refusal: string | null;
      switch (message.type) {
        case "create_collection":
          refusal = create(message);
          break;
        case "update_collection":
          refusal = update(message);
          break;
        case "delete_collection":
          refusal = remove(message.id);
          break;
        case "move_project":
          refusal = move(message.projectId, message.collectionId ?? null);
          break;
      }
      if (!refusal) host.changed();
      return refusal;
    },
  };
}

export type Collections = ReturnType<typeof createCollections>;
