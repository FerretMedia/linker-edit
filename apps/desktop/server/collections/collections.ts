import {
  PARENT_KINDS,
  type ClientMessage,
  type CollectionIcon,
  type CollectionKind,
  type CollectionMessage,
  type CollectionSummary,
  type IconColor,
  type LinkerOrg,
  type SkillImportResult,
} from "../../shared/protocol.ts";
import { COLLECTION_LABEL, parentWords } from "../../src/lib/labels.ts";
import type { ProjectCollection, SavedProject } from "../projects/store.ts";
import { randomUUID } from "node:crypto";
import * as path from "node:path";
import { agentRules, AUTONOMOUS_RULES, LEARNING_RULES, PROJECT_NOTES_FILE, PROJECT_RULES, readProjectNotes, setupRules, type RuleSource } from "../rules.ts";
import { formatBytes, SKILL_IMPORT_LIMITS } from "../../shared/skillImport.ts";
import { parseSkillImport } from "../skills/import.ts";
import { listSkills, removeSkill, savedSkills, skillsSection, writeSkills, type SkillRoot } from "../skills/skills.ts";
import { exampleName, RULES_FILE, skillBody, type CollectionStore, type SavedCollection } from "./store.ts";

/** What the brands, sub-brands and series need from the app around them. */
export interface CollectionsHost {
  store: CollectionStore;
  /** Linker-Edit's skills, which every project gets. */
  skills: SkillRoot[];
  /** Where your own skills are, which every project gets too. */
  ownSkills?: string;
  projects(): SavedProject[];
  /** The Linker orgs of the person signed in, as Linker last listed them; null when nobody is signed in. */
  orgs(): LinkerOrg[] | null;
  /** Saves the project in its new brand, sub-brand or series, or in none; commitMessage is for its edit history. */
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

/** A collection's own skills, next to its SKILL.md. */
const SKILLS_DIR = "skills";
const KINDS = Object.keys(PARENT_KINDS) as CollectionKind[];
const MAX_NAME = 80;
// SKILL.md is for a style guide, not a book; this keeps a paste accident out of every prompt.
const MAX_RULES = 20_000;

const isMessage = (message: ClientMessage): message is CollectionMessage =>
  ["create_collection", "update_collection", "delete_collection", "move_project", "remove_lesson", "remove_example", "remove_skill"].includes(message.type);

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

/** A collection and what it is in, root first: its brand, then its sub-brand, then itself. */
function lineage(collections: Map<string, SavedCollection>, collection: SavedCollection | undefined): SavedCollection[] {
  const chain: SavedCollection[] = [];
  // PARENT_KINDS only points up, so this ends within its few levels.
  for (let at = collection; at; at = at.parentId ? collections.get(at.parentId) : undefined) chain.unshift(at);
  return chain;
}

/** Root first: the brand, then the sub-brand and the series when there are any. Empty for a project in none. */
export function collectionChain(collections: Map<string, SavedCollection>, project: SavedProject): SavedCollection[] {
  return lineage(collections, project.collection ? collections.get(project.collection.id) : undefined);
}

/** Why a collection of this kind can't go in one of parentKind (null: in nothing); null when it can. */
export function parentRefusal(kind: CollectionKind, parentKind: CollectionKind | null): string | null {
  if (PARENT_KINDS[kind].includes(parentKind)) return null;
  const { lower } = COLLECTION_LABEL[kind];
  return PARENT_KINDS[kind].includes(null) ? `A ${lower} can't go inside anything.` : `A ${lower} can only go in a ${parentWords(kind)}.`;
}

/**
 * Brands, sub-brands and series: groups above projects, each with its own rules for the agent. A brand is a
 * Linker org; inside it go sub-brands and series as PARENT_KINDS allows, and projects in any of them.
 */
export function createCollections(host: CollectionsHost) {
  const { store } = host;
  const collections = new Map<string, SavedCollection>();
  for (const collection of store.load()) {
    if (KINDS.includes(collection.kind)) collections.set(collection.id, collection);
  }
  // One whose parent went missing or can't hold it waits under "Not in a brand" rather than disappearing.
  for (const collection of collections.values()) {
    const parent = collection.parentId ? collections.get(collection.parentId) : undefined;
    if (collection.parentId && (!parent || parentRefusal(collection.kind, parent.kind))) collection.parentId = null;
  }

  const label = (collection: SavedCollection) => `${COLLECTION_LABEL[collection.kind].one} "${collection.name}"`;
  const GONE = `That ${COLLECTION_LABEL.brand.lower}, ${COLLECTION_LABEL.subbrand.lower} or ${COLLECTION_LABEL.series.lower} is gone.`;

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

  /** Whether this project's agent makes the first cut without asking: its own choice, else its nearest collection's. */
  function autonomous(project: SavedProject): boolean {
    return project.autonomous ?? collectionChain(collections, project).at(-1)?.autonomous === true;
  }

  function skillsDir(id: string): string | null {
    const file = store.rulesFile(id);
    return file && path.join(path.dirname(file), SKILLS_DIR);
  }

  const skillRootsOf = (chain: SavedCollection[]): SkillRoot[] =>
    chain.flatMap((part) => {
      const dir = skillsDir(part.id);
      return dir ? [{ label: label(part), dir }] : [];
    });

  /** The skills every project gets: Linker-Edit's, then your own. */
  const sharedRoots: SkillRoot[] = [...host.skills, ...(host.ownSkills ? [{ label: "Your own", dir: host.ownSkills }] : [])];

  /** Where this project's skills are, most general first. */
  const skillRoots = (project: SavedProject): SkillRoot[] => [...sharedRoots, ...skillRootsOf(collectionChain(collections, project))];

  /** Your own skills' folder for null, else the collection's; null when it's gone. */
  const skillsOf = (id: string | null): string | null => (id === null ? (host.ownSkills ?? null) : collections.has(id) ? skillsDir(id) : null);

  /** Why nothing new can go in this collection's brand: the person isn't in its Linker org, or isn't signed in. */
  function closedRefusal(collection: SavedCollection): string | null {
    const root = lineage(collections, collection)[0]!;
    const orgs = host.orgs();
    if (root.kind !== "brand" || orgs?.some((org) => org.id === root.orgId)) return null;
    return orgs
      ? `You're not in "${root.name}" on Linker any more, so it takes nothing new.`
      : `Sign in to Linker to put things in the ${COLLECTION_LABEL.brand.lower} "${root.name}".`;
  }

  /** Why a collection of this kind can't go in parentId; null when it can. */
  function placeRefusal(kind: CollectionKind, parentId: string | null): string | null {
    const parent = parentId === null ? null : collections.get(parentId);
    if (parent === undefined) return GONE;
    return parentRefusal(kind, parent?.kind ?? null) ?? (parent && closedRefusal(parent));
  }

  /** The collection a project can go in, or why it can't: it's gone, or its brand is closed to the person. */
  function place(collectionId: string): { collection: SavedCollection } | { refusal: string } {
    const collection = collections.get(collectionId);
    if (!collection) return { refusal: GONE };
    const refusal = closedRefusal(collection);
    return refusal ? { refusal } : { collection };
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
    if (message.kind === "brand") return `${COLLECTION_LABEL.brand.many} are your Linker orgs: make one in Linker and it shows up here.`;
    if (!KINDS.includes(message.kind)) return `Pick a ${COLLECTION_LABEL.subbrand.lower} or a ${COLLECTION_LABEL.series.lower}.`;
    const name = cleanName(message.name);
    if (!name) return "Give it a name.";
    const parentId = message.parentId ?? null;
    const refusal = placeRefusal(message.kind, parentId);
    if (refusal) return refusal;
    const icon = isCollectionIcon(message.icon) ? message.icon : defaultIcon(name);
    const collection = store.create({ kind: message.kind, name, parentId, orgId: null, icon, rules: "" });
    collections.set(collection.id, collection);
    return null;
  }

  function update(message: Extract<CollectionMessage, { type: "update_collection" }>): string | null {
    const current = collections.get(message.id);
    if (!current) return null;
    const next = { ...current };
    if (message.name !== undefined) {
      if (current.kind === "brand") return `A ${COLLECTION_LABEL.brand.lower}'s name comes from Linker. Change it there.`;
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
    if (message.parentId !== undefined && message.parentId !== current.parentId) {
      const refusal = placeRefusal(next.kind, message.parentId);
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
    if (collection.kind === "brand") return `${label(collection)} is a Linker org, so it stays.`;
    const inside = [...collections.values()].filter((other) => other.parentId === id);
    const inIt = host.projects().filter((project) => project.collection?.id === id);
    const projects = inIt.filter((project) => !project.setup).length;
    if (inside.length || projects) {
      const what = [
        ...KINDS.flatMap((kind) => {
          const count = inside.filter((other) => other.kind === kind).length;
          return count ? [`${count} ${count === 1 ? COLLECTION_LABEL[kind].lower : COLLECTION_LABEL[kind].lowerMany}`] : [];
        }),
        ...(projects ? [`${projects} project${projects === 1 ? "" : "s"}`] : []),
      ].join(" and ");
      return `${label(collection)} still has ${what} in it. Move ${inside.length + projects === 1 ? "it" : "them"} out first.`;
    }
    collections.delete(id);
    store.remove(id);
    // Its setup threads go with it; their folder stays on disk like any project's.
    for (const project of inIt) host.removeProject(project.id);
    return null;
  }

  /**
   * The collection a brief tool writes to: one of the project's chain by kind (the nearest by default), or a
   * series by name next to the project's own, made by save_brief when there is none.
   */
  function target(project: SavedProject, input: Record<string, unknown>): { collection: SavedCollection } | { create: string; parentId: string } | { error: string } {
    const chain = collectionChain(collections, project);
    const nearest = chain.at(-1);
    if (!nearest) {
      return {
        error: `This project isn't in a ${COLLECTION_LABEL.brand.lower}, ${COLLECTION_LABEL.subbrand.lower} or ${COLLECTION_LABEL.series.lower}, so it has no brief. The user puts it in one in the project's settings.`,
      };
    }
    const kind = KINDS.find((candidate) => candidate === input.for) ?? nearest.kind;
    const own = chain.find((part) => part.kind === kind);
    if (kind !== "series") {
      return own ? { collection: own } : { error: `This project isn't in a ${COLLECTION_LABEL[kind].lower}. Pass for: "${nearest.kind}".` };
    }
    const name = cleanName(input.series);
    if (!name) return own ? { collection: own } : { error: `Name the ${COLLECTION_LABEL.series.lower} in series.` };
    if (own && own.name.toLowerCase() === name.toLowerCase()) return { collection: own };
    // Another series goes next to the project's own, in its brand or sub-brand.
    const home = chain.findLast((part) => part.kind !== "series");
    if (!home) {
      return { error: `This project's ${COLLECTION_LABEL.series.lower} isn't in a ${parentWords("series")}, so it can only change its own.` };
    }
    const found = [...collections.values()].find(
      (candidate) => candidate.kind === "series" && candidate.parentId === home.id && candidate.name.toLowerCase() === name.toLowerCase(),
    );
    if (found) return { collection: found };
    const closed = closedRefusal(home);
    return closed ? { error: closed } : { create: name, parentId: home.id };
  }

  function saveRules(found: { collection: SavedCollection } | { create: string; parentId: string }, rules: string): SavedCollection {
    if ("create" in found) {
      const created = store.create({ kind: "series", name: found.create, parentId: found.parentId, orgId: null, icon: defaultIcon(found.create), rules });
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
    if (project.collection?.id === collectionId) return null;
    const placed = place(collectionId);
    if ("refusal" in placed) return placed.refusal;
    const { collection } = placed;
    host.setCollection(projectId, { id: collection.id, name: collection.name }, `Put the project in ${label(collection)}`);
    return null;
  }

  return {
    /** Brands first, then sub-brands, then series, each in the order they were made. */
    summaries(): CollectionSummary[] {
      const projects = host.projects();
      const orgs = host.orgs();
      return [...collections.values()]
        .sort((a, b) => KINDS.indexOf(a.kind) - KINDS.indexOf(b.kind))
        .map((collection) => ({
          id: collection.id,
          kind: collection.kind,
          name: collection.name,
          parentId: collection.parentId,
          org: collection.orgId === null ? null : orgSummary(collection.orgId, orgs),
          icon: collection.icon,
          rules: collection.rules,
          rulesFile: store.rulesFile(collection.id) ?? "",
          projectIds: projects.filter((project) => project.collection?.id === collection.id && !project.setup).map((project) => project.id),
          autonomous: collection.autonomous,
          lessons: collection.lessons,
          examples: store.examples(collection.id),
          skills: savedSkills(skillsDir(collection.id) ?? ""),
          setupProjectId: projects.find((project) => project.setup && project.collection?.id === collection.id)?.id ?? null,
          edits: collection.edits,
        }));
    },

    /** Linker-Edit's skills. */
    sharedSkills: () => listSkills(host.skills).map(({ name, description }) => ({ name, description })),

    ownSkills: () => (host.ownSkills ? savedSkills(host.ownSkills) : []),

    /** import_skills: the skills in dropped files, written to your own (id null) or a collection's. */
    importSkills(id: string | null, dropped: { path: string; base64: string }[]): SkillImportResult | { error: string } {
      const dir = skillsOf(id);
      if (!dir) return { error: id === null ? "This Linker-Edit keeps no skills of your own." : GONE };
      const files = Array.isArray(dropped)
        ? dropped.flatMap((file) => (typeof file?.path === "string" && typeof file.base64 === "string" ? [{ path: file.path, bytes: Buffer.from(file.base64, "base64") }] : []))
        : [];
      const size = files.reduce((sum, file) => sum + file.bytes.byteLength, 0);
      if (size > SKILL_IMPORT_LIMITS.dropBytes) return { error: `That's ${formatBytes(size)}. Drop up to ${formatBytes(SKILL_IMPORT_LIMITS.dropBytes)} at a time.` };
      // A skill of the same name as one above it adds to that one, so it needs no description.
      const above = id === null ? host.skills : [...sharedRoots, ...skillRootsOf(lineage(collections, collections.get(id)).slice(0, -1))];
      const { skills, skipped } = parseSkillImport(files, new Set(listSkills(above).map((skill) => skill.name)));
      const written = writeSkills(dir, skills);
      if (written.imported.length) host.changed();
      return { ...written, skipped: [...skipped, ...written.skipped] };
    },

    get: (id: string): SavedCollection | undefined => collections.get(id),

    /** The project's brand, sub-brand and series, root first. */
    chain: (project: SavedProject): SavedCollection[] => collectionChain(collections, project),

    place,

    /** Makes a brand for each Linker org that has none yet, and follows a rename in Linker. A brand is never deleted here. */
    syncBrands() {
      let changed = false;
      for (const org of host.orgs() ?? []) {
        const name = cleanName(org.name) ?? "Linker org";
        const brand = [...collections.values()].find((collection) => collection.kind === "brand" && collection.orgId === org.id);
        if (!brand) {
          const made = store.create({ kind: "brand", name, parentId: null, orgId: org.id, icon: defaultIcon(name), rules: "" });
          collections.set(made.id, made);
        } else if (brand.name !== name) {
          const renamed = { ...brand, name };
          collections.set(renamed.id, renamed);
          store.save(renamed);
          renameInProjects(renamed);
        } else continue;
        changed = true;
      }
      if (changed) host.changed();
    },

    /**
     * The agent's rules for this project: the editing rules, then those of its brand, sub-brand, series and its
     * own, each with the lessons learned there. A setup thread gets the setup assistant's rules instead.
     */
    rulesFor(project: SavedProject): string {
      const chain = collectionChain(collections, project);
      const nearest = chain.at(-1);
      if (project.setup && nearest) {
        const parts = [...chain, ...[...collections.values()].filter((other) => other.parentId === nearest.id)];
        const state = parts.map((part) => `## ${label(part)}\n\n${briefText(part) || "Nothing yet."}`).join("\n\n");
        return setupRules(label(nearest), state);
      }
      // Like a repository inside folders with instructions of their own: each heading names the file it is from.
      const sources: RuleSource[] = chain.map((part) => ({ heading: `${label(part)} (${store.rulesFile(part.id) ?? RULES_FILE})`, text: briefText(part) }));
      sources.push({ heading: `This project (${PROJECT_NOTES_FILE} in the working folder)`, text: readProjectNotes(project.dir) });
      const skills = skillsSection(listSkills(skillRoots(project)));
      return agentRules(sources, [PROJECT_RULES, ...(autonomous(project) ? [AUTONOMOUS_RULES] : []), ...(nearest ? [LEARNING_RULES] : []), ...(skills ? [skills] : [])]);
    },

    skillRoots,

    autonomous,

    /** save_brief: new rules for one of the project's collections or a series, shown to the user as a change. */
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
      if ("create" in found) return { error: `There is no ${COLLECTION_LABEL.series.lower} "${found.create}" next to this project's.` };
      const rule = text(input.rule);
      const feedback = text(input.feedback);
      if (!rule || !feedback) return { error: "Pass the rule, and the user's own words it comes from in feedback." };
      if (rule.length > MAX_LESSON) return { error: `A lesson is one short rule, up to ${MAX_LESSON} characters. For more, change the rules with save_brief.` };
      const { collection } = found;
      if (collection.lessons.length >= MAX_LESSONS) {
        return { error: `${label(collection)} has ${MAX_LESSONS} lessons already. Fold them into its rules with save_brief; the user removes lessons on the Brands page.` };
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
      const nearest = collectionChain(collections, project).at(-1);
      if (!nearest) return;
      const edit = { at: new Date().toISOString(), project: projectName, rounds };
      change(nearest.id, (current) => ({ ...current, edits: [edit, ...current.edits].slice(0, MAX_EDITS) }));
      host.changed();
    },

    /** Handles the brand, sub-brand and series messages; others are left alone. Returns why it was refused, if it was. */
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
        case "remove_skill": {
          const dir = skillsOf(message.id ?? null);
          if (dir) refusal = removeSkill(dir, message.name);
          break;
        }
      }
      if (!refusal) host.changed();
      return refusal;
    },
  };
}

/** A brand's org as clients see it: its picture, and whether Linker last listed the person in it. */
function orgSummary(orgId: string, orgs: LinkerOrg[] | null): CollectionSummary["org"] {
  const org = orgs?.find((candidate) => candidate.id === orgId);
  return { id: orgId, imageUrl: org?.imageUrl ?? null, member: !!org };
}

export type Collections = ReturnType<typeof createCollections>;
