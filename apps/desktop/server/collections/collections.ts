import type { ClientMessage, CollectionIcon, CollectionKind, CollectionMessage, CollectionSummary, IconColor } from "../../shared/protocol.ts";
import { COLLECTION_LABEL } from "../../src/lib/labels.ts";
import type { ProjectCollection, SavedProject } from "../projects/store.ts";
import { randomUUID } from "node:crypto";
import * as path from "node:path";
import { agentRules, AUTONOMOUS_RULES, LEARNING_RULES, PROJECT_NOTES_FILE, PROJECT_RULES, readProjectNotes, setupRules, type RuleSource } from "../rules.ts";
import { exampleName, RULES_FILE, skillBody, type CollectionStore, type SavedCollection } from "./store.ts";

/** What the workspaces and series need from the app around them. */
export interface CollectionsHost {
  store: CollectionStore;
  projects(): SavedProject[];
  /** Saves the project in its new workspace or series, or in none; commitMessage is for its edit history. */
  setCollection(projectId: string, collection: ProjectCollection | null, commitMessage: string): void;
  /** Takes a deleted collection's setup threads out of the app. */
  removeProject(projectId: string): void;
  /** Clients and the agents' rules need to see a change. */
  changed(): void;
}

/**
 * A change to the brief an agent proposes. The user sees `shows` and accepts or declines; apply() makes the
 * change and returns what the agent is told.
 */
export interface BriefProposal {
  shows: Record<string, unknown>;
  apply(): string;
}

const MAX_LESSONS = 60;
const MAX_LESSON = 400;
const MAX_FEEDBACK = 600;
const MAX_EXAMPLE = 12_000;
const MAX_EDITS = 30;

export const ICON_COLORS: readonly IconColor[] = ["gray", "red", "orange", "amber", "green", "teal", "blue", "violet", "pink"];

const KINDS: readonly CollectionKind[] = ["workspace", "series"];
const MAX_NAME = 80;
// SKILL.md is for a style guide, not a book; this keeps a paste accident out of every prompt.
const MAX_RULES = 20_000;

const isMessage = (message: ClientMessage): message is CollectionMessage =>
  ["create_collection", "update_collection", "delete_collection", "move_project", "remove_lesson", "remove_example"].includes(message.type);

const text = (value: unknown) => (typeof value === "string" ? value.trim() : "");

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

  /** A collection's part of the brief as agents read it: its rules, its lessons, and where its example notes are. */
  function briefText(collection: SavedCollection): string {
    const examples = store.examples(collection.id);
    return [
      collection.rules.trim(),
      collection.lessons.length
        ? `Lessons from the user's feedback, each accepted by the user. They win over the rules above where they disagree:\n${collection.lessons.map((lesson) => `- ${lesson.rule}`).join("\n")}`
        : "",
      examples.length
        ? `Notes on ${examples.length} example video${examples.length === 1 ? "" : "s"} are in ${path.dirname(examples[0]!.file)} (${examples.map((example) => example.name).join(", ")}). Read one when you need a reference.`
        : "",
    ]
      .filter(Boolean)
      .join("\n\n");
  }

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
    if (typeof message.autonomous === "boolean") next.autonomous = message.autonomous;
    collections.set(next.id, next);
    store.save(next);
    if (next.name !== current.name) renameInProjects(next);
    return null;
  }

  function remove(id: string): string | null {
    const collection = collections.get(id);
    if (!collection) return null;
    const series = [...collections.values()].filter((other) => other.parentId === id).length;
    const inIt = host.projects().filter((project) => project.collection?.id === id);
    const projects = inIt.filter((project) => !project.setup).length;
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
    // Its setup threads go with it; their folder stays on disk like any project's.
    for (const project of inIt) host.removeProject(project.id);
    return null;
  }

  /** The collection a brief tool writes to: the project's workspace or series, or a series of its workspace by name. */
  function target(project: SavedProject, input: Record<string, unknown>): { collection: SavedCollection } | { create: string; parentId: string } | { error: string } {
    const { workspace, series } = collectionChain(collections, project);
    const where = input.for === "workspace" || input.for === "series" ? input.for : series ? "series" : "workspace";
    if (!workspace && !series) {
      return { error: `This project isn't in a ${COLLECTION_LABEL.workspace.lower} or ${COLLECTION_LABEL.series.lower}, so it has no brief. The user puts it in one in the project's settings.` };
    }
    if (where === "workspace") {
      return workspace ? { collection: workspace } : { error: `This project's ${COLLECTION_LABEL.series.lower} isn't in a ${COLLECTION_LABEL.workspace.lower}. Pass for: "series".` };
    }
    const name = cleanName(input.series);
    if (!name) return series ? { collection: series } : { error: `Name the ${COLLECTION_LABEL.series.lower} in series.` };
    if (series && series.name.toLowerCase() === name.toLowerCase()) return { collection: series };
    if (!workspace) return { error: `This project's ${COLLECTION_LABEL.series.lower} isn't in a ${COLLECTION_LABEL.workspace.lower}, so it can only change its own.` };
    const found = [...collections.values()].find(
      (candidate) => candidate.kind === "series" && candidate.parentId === workspace.id && candidate.name.toLowerCase() === name.toLowerCase(),
    );
    return found ? { collection: found } : { create: name, parentId: workspace.id };
  }

  function saveRules(found: { collection: SavedCollection } | { create: string; parentId: string }, rules: string): SavedCollection {
    if ("create" in found) {
      const created = store.create({ kind: "series", name: found.create, parentId: found.parentId, icon: defaultIcon(found.create), rules });
      collections.set(created.id, created);
      return created;
    }
    const next = { ...collections.get(found.collection.id)!, rules };
    collections.set(next.id, next);
    store.save(next);
    return next;
  }

  function change(id: string, update: (collection: SavedCollection) => SavedCollection): SavedCollection | null {
    const current = collections.get(id);
    if (!current) return null;
    const next = update(current);
    collections.set(id, next);
    store.save(next);
    return next;
  }

  function move(projectId: string, collectionId: string | null): string | null {
    const project = host.projects().find((candidate) => candidate.id === projectId);
    if (!project) return null;
    if (project.setup) return "Setup threads stay with what they set up.";
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
          projectIds: projects.filter((project) => project.collection?.id === collection.id && !project.setup).map((project) => project.id),
          autonomous: collection.autonomous,
          lessons: collection.lessons,
          examples: store.examples(collection.id),
          setupProjectId: projects.find((project) => project.setup && project.collection?.id === collection.id)?.id ?? null,
          edits: collection.edits,
        }));
    },

    get: (id: string): SavedCollection | undefined => collections.get(id),

    /**
     * The agent's rules for this project: the editing rules, then its workspace's, its series' and its own, each
     * with the lessons learned there. A setup thread gets the setup assistant's rules instead.
     */
    rulesFor(project: SavedProject): string {
      const { workspace, series } = collectionChain(collections, project);
      if (project.setup) {
        const subject = series ?? workspace;
        if (subject) {
          const parts = subject.kind === "workspace" ? [subject, ...[...collections.values()].filter((other) => other.parentId === subject.id)] : [workspace, subject];
          const state = parts
            .flatMap((part) => (part ? [`## ${label(part)}\n\n${briefText(part) || "Nothing yet."}`] : []))
            .join("\n\n");
          return setupRules(label(subject), state);
        }
      }
      const sources: RuleSource[] = [];
      // Like a repository inside folders with instructions of their own: each heading names the file it is from.
      for (const part of [workspace, series]) {
        if (part) sources.push({ heading: `${label(part)} (${store.rulesFile(part.id) ?? RULES_FILE})`, text: briefText(part) });
      }
      sources.push({ heading: `This project (${PROJECT_NOTES_FILE} in the working folder)`, text: readProjectNotes(project.dir) });
      const nearest = series ?? workspace;
      return agentRules(sources, [PROJECT_RULES, ...(nearest ? [...(nearest.autonomous ? [AUTONOMOUS_RULES] : []), LEARNING_RULES] : [])]);
    },

    /** Whether this project's agent makes the first cut without asking. */
    autonomous(project: SavedProject): boolean {
      const { workspace, series } = collectionChain(collections, project);
      return (series ?? workspace)?.autonomous === true;
    },

    /** save_brief: new rules for the project's workspace or a series, shown to the user as a change. */
    proposeBrief(project: SavedProject, input: Record<string, unknown>): BriefProposal | { error: string } {
      const found = target(project, input);
      if ("error" in found) return found;
      const rules = typeof input.rules === "string" ? skillBody(input.rules).trim() : "";
      if (!rules) return { error: "Pass the full new rules in rules." };
      if (rules.length > MAX_RULES) return { error: `Rules can be up to ${MAX_RULES.toLocaleString("en-US")} characters; these are ${rules.length.toLocaleString("en-US")}. Make them shorter.` };
      const name = "create" in found ? `New ${COLLECTION_LABEL.series.lower} "${found.create}"` : label(found.collection);
      return {
        shows: { for: name, summary: text(input.summary).slice(0, 600), was: "create" in found ? "" : found.collection.rules, rules },
        apply() {
          const saved = saveRules(found, `${rules}\n`);
          host.changed();
          return `Saved the rules of ${label(saved)}.`;
        },
      };
    },

    /** add_lesson: one rule learned from feedback, shown to the user next to their own words. */
    proposeLesson(project: SavedProject, input: Record<string, unknown>, projectName: string): BriefProposal | { error: string } {
      const found = target(project, input);
      if ("error" in found) return found;
      if ("create" in found) return { error: `There is no ${COLLECTION_LABEL.series.lower} "${found.create}" in this ${COLLECTION_LABEL.workspace.lower}.` };
      const rule = text(input.rule);
      const feedback = text(input.feedback);
      if (!rule || !feedback) return { error: "Pass the rule, and the user's own words it comes from in feedback." };
      if (rule.length > MAX_LESSON) return { error: `A lesson is one short rule, up to ${MAX_LESSON} characters. For more, change the rules with save_brief.` };
      const { collection } = found;
      if (collection.lessons.length >= MAX_LESSONS) {
        return { error: `${label(collection)} has ${MAX_LESSONS} lessons already. Fold them into its rules with save_brief; the user removes lessons in Settings.` };
      }
      return {
        shows: { for: label(collection), feedback: feedback.slice(0, MAX_FEEDBACK), rule },
        apply() {
          const lesson = { id: randomUUID(), rule, feedback: feedback.slice(0, MAX_FEEDBACK), addedAt: new Date().toISOString(), project: projectName };
          change(collection.id, (current) => ({ ...current, lessons: [...current.lessons, lesson] }));
          host.changed();
          return `Added the lesson to ${label(collection)}.`;
        },
      };
    },

    /** save_example: the agent's notes on one example video. They are its own notes, so nobody is asked. */
    saveExample(project: SavedProject, input: Record<string, unknown>): string {
      const found = target(project, input);
      if ("error" in found) throw new Error(found.error);
      if ("create" in found) throw new Error(`There is no ${COLLECTION_LABEL.series.lower} "${found.create}" yet. Save its rules with save_brief first, or leave series out.`);
      const name = text(input.name);
      const notes = text(input.notes);
      if (!name || !notes) throw new Error("Pass the video's name and your notes.");
      if (notes.length > MAX_EXAMPLE) throw new Error(`Notes on one example can be up to ${MAX_EXAMPLE.toLocaleString("en-US")} characters.`);
      const file = store.saveExample(found.collection.id, name, `# ${name}\n\n${notes}\n`);
      if (!file) throw new Error(`${label(found.collection)} is gone.`);
      host.changed();
      return `Saved the notes on "${exampleName(name)}" for ${label(found.collection)}: ${file}`;
    },

    /** An AI timeline of this project became its main one, after this many rounds of feedback. */
    recordEdit(project: SavedProject, projectName: string, rounds: number) {
      const { workspace, series } = collectionChain(collections, project);
      const nearest = series ?? workspace;
      if (!nearest) return;
      const edit = { at: new Date().toISOString(), project: projectName, rounds };
      change(nearest.id, (current) => ({ ...current, edits: [edit, ...current.edits].slice(0, MAX_EDITS) }));
      host.changed();
    },

    /** Handles the workspace and series messages; others are left alone. Returns why it was refused, if it was. */
    handle(message: ClientMessage): string | null {
      if (!isMessage(message)) return null;
      let refusal: string | null = null;
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
        case "remove_lesson":
          change(message.id, (current) => ({ ...current, lessons: current.lessons.filter((lesson) => lesson.id !== message.lessonId) }));
          break;
        case "remove_example":
          if (typeof message.name === "string") store.removeExample(message.id, message.name);
          break;
      }
      if (!refusal) host.changed();
      return refusal;
    },
  };
}

export type Collections = ReturnType<typeof createCollections>;
