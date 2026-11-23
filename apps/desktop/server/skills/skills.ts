// Skills say how a kind of work is done. Agents get a list of them in their rules, and read one with read_skill
// when the work needs it.
import * as fs from "node:fs";
import * as path from "node:path";
import { FRONT_MATTER, skillBody } from "../collections/store.ts";

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
    "Read a skill from the list in your rules: how that kind of work is done here. Read it before you plan or start the work. A skill can name other files of its own; pass one as file.",
  inputSchema: {
    type: "object",
    properties: {
      name: { type: "string", description: "The skill's name, as listed." },
      file: { type: "string", description: 'A file the skill names, such as "interview.md".' },
    },
    required: ["name"],
  },
  annotations: { readOnlyHint: true },
};

const SKILL_FILE = "SKILL.md";
const SKILL_NAME = /^[a-z0-9][a-z0-9-]{0,39}$/;
const DETAIL_FILE = /^[a-z0-9][a-z0-9-]*\.md$/;
const MAX_TEXT = 20_000;
const MAX_DESCRIPTION = 300;

function readText(file: string): string | null {
  try {
    return fs.readFileSync(file, "utf8");
  } catch {
    return null;
  }
}

/** The front matter's description: on its line, or on the indented lines below it, as YAML allows. */
function description(text: string): string {
  const found = /^description:[ \t]*(.*(?:\r?\n[ \t]+.*)*)/m.exec(FRONT_MATTER.exec(text)?.[1] ?? "")?.[1] ?? "";
  const value = found.replace(/^[>|][+-]?(?=\s|$)/, "").replace(/\s+/g, " ").trim();
  const [, quote, inner] = /^(["'])(.*)\1$/.exec(value) ?? [];
  return (quote === '"' ? inner!.replaceAll('\\"', '"') : quote ? inner!.replaceAll("''", "'") : value).slice(0, MAX_DESCRIPTION);
}

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

/** The list of skills for the agent's rules; "" when there are none. */
export function skillsSection(skills: Skill[]): string {
  if (skills.length === 0) return "";
  return `# Skills

A skill says how a kind of work is done: Linker-Edit's way, then what the user and this project's workspace and series add. Before you plan or change anything, read every skill the task needs with ${READ_SKILL}, and follow it. The user's rules above win over a skill, and no skill lifts rules 1 to 8.

${skills.map(({ name, description: about, sources }) => `- ${name}: ${about} (${sources.map((source) => source.label).join(", ")})`).join("\n")}`;
}

/** A skill's text from every root that has it, or one of its other files from the most specific root that has that. */
export function readSkill(roots: SkillRoot[], name: string, file?: string): string {
  const skills = listSkills(roots);
  const skill = skills.find((candidate) => candidate.name === name);
  if (!skill) throw new Error(skills.length ? `There is no skill "${name}". There are: ${skills.map((candidate) => candidate.name).join(", ")}.` : "There are no skills here.");
  if (file) {
    if (!DETAIL_FILE.test(file)) throw new Error(`"${file}" isn't a file name a skill can have, such as "interview.md".`);
    for (const { dir } of roots.toReversed()) {
      const text = readText(path.join(dir, name, file));
      if (text !== null) return capped(text);
    }
    throw new Error(`The skill "${name}" has no file "${file}".`);
  }
  const parts = skill.sources.flatMap((source) => {
    const text = readText(source.file);
    return text === null ? [] : [`## ${source.label} (${source.file})\n\n${capped(skillBody(text).trim())}`];
  });
  return [...(parts.length > 1 ? ["Later parts add to earlier ones and win where they disagree."] : []), ...parts].join("\n\n");
}
