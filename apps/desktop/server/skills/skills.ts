// Skills say how a kind of work is done. Agents get a list of them in their rules, and read one with read_skill
// when the work needs it.
import { randomUUID } from "node:crypto";
import * as fs from "node:fs";
import * as path from "node:path";
import type { SavedSkill, SkillImportResult } from "../../shared/protocol.ts";
import { FRONT_MATTER, skillBody } from "../collections/store.ts";
import type { ImportedSkill } from "./import.ts";

/** A folder of skills, and whose they are. */
export interface SkillRoot {
  /** As in 'Series "Podcast"'. */
  label: string;
  dir: string;
}

/** A skill of a project: every root that has one of this name adds to it, most general first. */
export interface Skill {
  name: string;
  description: string;
  sources: { label: string; file: string }[];
}

export const READ_SKILL = "read_skill";

export const READ_SKILL_TOOL = {
  name: READ_SKILL,
  description:
    "Read a skill from the list in your rules: how that kind of work is done here. Read it before you plan or start the work. A skill can name other files in its folder, such as references/interview.md; pass that path as file.",
  inputSchema: {
    type: "object",
    properties: {
      name: { type: "string", description: "The skill's name, as listed." },
      file: { type: "string", description: 'A file in the skill\'s folder, by its path there, such as "references/interview.md".' },
    },
    required: ["name"],
  },
  annotations: { readOnlyHint: true },
};

export const SKILL_FILE = "SKILL.md";
export const SKILL_NAME = /^[a-z0-9][a-z0-9-]{0,39}$/;
// A folder or file name in a skill's folder: not hidden, so never "." or "..", and nothing a path could escape with.
const SAFE_PART = /^[^.\\/:*?"<>|\x00-\x1f][^\\/:*?"<>|\x00-\x1f]*$/;
const MAX_TEXT = 20_000;
export const MAX_DESCRIPTION = 300;

function readText(file: string): string | null {
  try {
    return fs.readFileSync(file, "utf8");
  } catch {
    return null;
  }
}

/** A top-level field of front matter: its line, and the indented lines below it, as YAML allows. */
export const frontMatterField = (key: "name" | "description") => new RegExp(`^${key}:[ \\t]*(.*(?:\\r?\\n[ \\t]+.*)*)`, "m");

/** A field of a skill's front matter as one line of plain text; "" when it has none. */
export function frontMatterValue(text: string, key: "name" | "description"): string {
  const found = frontMatterField(key).exec(FRONT_MATTER.exec(text)?.[1] ?? "")?.[1] ?? "";
  const value = found.replace(/^[>|][+-]?(?=\s|$)/, "").replace(/\s+/g, " ").trim();
  const [, quote, inner] = /^(["'])(.*)\1$/.exec(value) ?? [];
  return quote === '"' ? inner!.replaceAll('\\"', '"') : quote ? inner!.replaceAll("''", "'") : value;
}

const description = (text: string) => frontMatterValue(text, "description").slice(0, MAX_DESCRIPTION);

const capped = (text: string) => (text.length <= MAX_TEXT ? text : `${text.slice(0, MAX_TEXT)}\n\n(The rest is left out: a skill can be up to ${MAX_TEXT.toLocaleString("en-US")} characters.)`);

/** The skills in these roots, most general root first. The most specific description wins. */
export function listSkills(roots: SkillRoot[]): Skill[] {
  const skills = new Map<string, Skill>();
  for (const { label, dir } of roots) {
    let names: string[] = [];
    try {
      names = fs.readdirSync(dir).filter((name) => SKILL_NAME.test(name));
    } catch {
      continue;
    }
    for (const name of names) {
      const file = path.join(dir, name, SKILL_FILE);
      const text = readText(file);
      if (text === null) continue;
      const skill = skills.get(name) ?? { name, description: "", sources: [] };
      skills.set(name, { name, description: description(text) || skill.description, sources: [...skill.sources, { label, file }] });
    }
  }
  return [...skills.values()].sort((a, b) => a.name.localeCompare(b.name));
}

/** The skills in one folder, as the app lists them. */
export function savedSkills(dir: string): SavedSkill[] {
  return listSkills([{ label: "", dir }]).map(({ name, description: about, sources }) => ({ name, description: about, file: sources[0]!.file }));
}

// A skill being written, or one being replaced, while an import runs. Hidden, so never listed as a skill.
const IMPORTING = ".importing-";

/**
 * Writes imported skills into dir. Each replaces the folder of a skill of its name whole: it is written next to it
 * first and renamed into place, so a failed import leaves the skill as it was.
 */
export function writeSkills(dir: string, skills: ImportedSkill[]): SkillImportResult {
  const result: SkillImportResult = { imported: [], replaced: [], skipped: [] };
  try {
    fs.mkdirSync(dir, { recursive: true });
    // What an import that stopped half way left behind.
    for (const entry of fs.readdirSync(dir)) if (entry.startsWith(IMPORTING)) fs.rmSync(path.join(dir, entry), { recursive: true, force: true });
  } catch (error) {
    result.skipped = skills.map((skill) => ({ path: skill.name, reason: `Couldn't write it: ${errorText(error)}` }));
    return result;
  }
  for (const skill of skills) {
    const target = path.join(dir, skill.name);
    const temp = path.join(dir, `${IMPORTING}${randomUUID()}`);
    const old = `${temp}-old`;
    try {
      for (const [file, text] of skill.files) {
        const full = path.join(temp, ...file.split("/"));
        fs.mkdirSync(path.dirname(full), { recursive: true });
        fs.writeFileSync(full, text);
      }
      const replacing = fs.existsSync(target);
      if (replacing) fs.renameSync(target, old);
      try {
        fs.renameSync(temp, target);
      } catch (error) {
        if (replacing) fs.renameSync(old, target);
        throw error;
      }
      fs.rmSync(old, { recursive: true, force: true });
      result.imported.push(skill.name);
      if (replacing) result.replaced.push(skill.name);
    } catch (error) {
      fs.rmSync(temp, { recursive: true, force: true });
      result.skipped.push({ path: skill.name, reason: `Couldn't write it: ${errorText(error)}` });
    }
  }
  return result;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Deletes a skill's folder, its other files included. Returns why it couldn't, or null. */
export function removeSkill(dir: string, name: unknown): string | null {
  if (typeof name !== "string" || !SKILL_NAME.test(name)) return null;
  try {
    fs.rmSync(path.join(dir, name), { recursive: true, force: true });
  } catch (error) {
    return `Couldn't remove the skill: ${errorText(error)}`;
  }
  return null;
}

/** The list of skills for the agent's rules; "" when there are none. */
export function skillsSection(skills: Skill[]): string {
  if (skills.length === 0) return "";
  return `# Skills

A skill says how a kind of work is done: Linker-Edit's way, then what the user and this project's brand, sub-brand and series add. Before you plan or change anything, read every skill the task needs with ${READ_SKILL}, and follow it. The user's rules above win over a skill, and no skill lifts rules 1 to 8.

${skills.map(({ name, description: about, sources }) => `- ${name}: ${about} (${sources.map((source) => source.label).join(", ")})`).join("\n")}`;
}

/** The paths of everything in a skill's folder, in any root, "/" between folders. */
function skillFiles(roots: SkillRoot[], name: string): string[] {
  return roots.flatMap(({ dir }) => {
    try {
      return fs.readdirSync(path.join(dir, name), { recursive: true, encoding: "utf8" }).map((file) => file.split(path.sep).join("/"));
    } catch {
      return [];
    }
  });
}

/** A skill's text from every root that has it, or one of its other files from the most specific root that has that. */
export function readSkill(roots: SkillRoot[], name: string, file?: string): string {
  const skills = listSkills(roots);
  const skill = skills.find((candidate) => candidate.name === name);
  // Agents ask for a skill's file as if it were a skill.
  const owner =
    skill || !SKILL_NAME.test(name)
      ? undefined
      : skills.find((other) => skillFiles(roots, other.name).some((found) => found === `${name}.md` || found.endsWith(`/${name}.md`)));
  if (owner) return readSkill(roots, owner.name, `${name}.md`);
  if (!skill) throw new Error(skills.length ? `There is no skill "${name}". There are: ${skills.map((candidate) => candidate.name).join(", ")}.` : "There are no skills here.");
  if (file) {
    if (!file.split("/").every((part) => SAFE_PART.test(part))) throw new Error(`"${file}" isn't a path in the skill's folder, such as "references/interview.md".`);
    // Named by its file name alone, while it is in a folder of the skill.
    const paths = file.includes("/") ? [file] : [file, ...skillFiles(roots, name).filter((found) => found.endsWith(`/${file}`))];
    for (const found of paths) {
      for (const { dir } of roots.toReversed()) {
        const text = readText(path.join(dir, name, ...found.split("/")));
        if (text === null) continue;
        if (text.includes("\0")) throw new Error(`"${found}" isn't a text file.`);
        return capped(text);
      }
    }
    throw new Error(`The skill "${name}" has no file "${file}".`);
  }
  const parts = skill.sources.flatMap((source) => {
    const text = readText(source.file);
    return text === null ? [] : [`## ${source.label} (${source.file})\n\n${capped(skillBody(text).trim())}`];
  });
  return [...(parts.length > 1 ? ["Later parts add to earlier ones and win where they disagree."] : []), ...parts].join("\n\n");
}
