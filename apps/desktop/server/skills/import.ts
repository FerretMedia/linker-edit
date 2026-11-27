// Skills people bring: one SKILL.md, a skill's folder, a folder of many skills, or a zip or .skill export of any of
// those. Files in, skills out; writeSkills puts them on disk.
import type { SkippedFile } from "../../shared/protocol.ts";
import { formatBytes, hiddenPart, SKILL_IMPORT_LIMITS as LIMITS } from "../../shared/skillImport.ts";
import { collectionSlug, FRONT_MATTER, skillBody } from "../collections/store.ts";
import { frontMatterField, frontMatterValue, MAX_DESCRIPTION, SKILL_FILE, SKILL_NAME } from "./skills.ts";
import { unzip } from "./zip.ts";

export interface DroppedFile {
  /** Where it was in what was dropped, such as "my-skills/color/SKILL.md". */
  path: string;
  bytes: Uint8Array;
}

export interface ImportedSkill {
  name: string;
  /** "" for one that only adds to a skill of its name above it. */
  description: string;
  /** Its files by their path in its folder, "/" between folders. Its SKILL.md has its name and description written in. */
  files: Map<string, string>;
}

const ARCHIVE = /\.(zip|skill)$/i;
// Notes about the skills next to them, as in a repository of skills.
const NOT_SKILLS = new Set(["readme", "license", "changelog", "contributing"]);
const UTF8 = new TextDecoder("utf-8", { fatal: true });

function decode(bytes: Uint8Array): string | null {
  try {
    const text = UTF8.decode(bytes);
    return text.includes("\0") ? null : text;
  } catch {
    return null;
  }
}

/** A name as SKILL_NAME allows it, from a name in front matter or a folder's; "" when nothing is left of it. */
function skillName(raw: string): string {
  const name = /[a-z0-9]/i.test(raw) ? collectionSlug(raw).slice(0, 40).replace(/-+$/, "") : "";
  return SKILL_NAME.test(name) ? name : "";
}

/** The first paragraph that isn't a heading, as a description for a file that has none. */
function firstParagraph(body: string): string {
  for (const block of body.split(/\r?\n[ \t]*\r?\n/)) {
    const text = block.trim();
    if (text && !/^(#|```|~~~|<!--|---)/.test(text)) return text.replace(/\s+/g, " ").slice(0, MAX_DESCRIPTION).trim();
  }
  return "";
}

const withoutField = (fields: string, key: "name" | "description") => fields.replace(new RegExp(`${frontMatterField(key).source}\\r?\\n?`, "m"), "");

/** SKILL.md with its name, and a description when it had none; its other front matter stays as it was. */
function withFrontMatter(text: string, name: string, added: string | null): string {
  const fields = FRONT_MATTER.exec(text)?.[1] ?? "";
  const others = (added ? withoutField(withoutField(fields, "name"), "description") : withoutField(fields, "name")).trim();
  const lines = [`name: ${name}`, ...(added ? [`description: '${added.replaceAll("'", "''")}'`] : []), ...(others ? [others] : [])];
  return `---\n${lines.join("\n")}\n---\n\n${skillBody(text)}`;
}

/**
 * The skills in dropped files. Zips and .skill files are unpacked first, into a folder named after them. Each
 * SKILL.md makes its folder a skill, with the files under it that aren't another skill's; a Markdown file in no skill
 * is a skill by itself. `above` are the skills a skill of the same name adds to, so it needs no description.
 */
export function parseSkillImport(dropped: DroppedFile[], above: ReadonlySet<string> = new Set()): { skills: ImportedSkill[]; skipped: SkippedFile[] } {
  const skipped: SkippedFile[] = [];
  const skip = (path: string, reason: string) => {
    if (!skipped.some((entry) => entry.path === path)) skipped.push({ path, reason });
  };
  const texts = new Map<string, string>();
  const zipped = { entries: 0, bytes: 0 };

  const add = (raw: string, bytes: Uint8Array, inZip: boolean) => {
    const parts = raw.replaceAll("\\", "/").split("/").filter((part) => part && part !== ".");
    const path = parts.join("/");
    if (!parts.length || parts.some((part) => part === ".." || part.includes(":"))) return skip(raw, "Its path points outside what was dropped.");
    const hidden = hiddenPart(path);
    if (hidden) return skip(hidden, "It's hidden.");
    if (!inZip && ARCHIVE.test(path)) {
      try {
        const zip = unzip(bytes, { entries: LIMITS.zipEntries - zipped.entries, bytes: LIMITS.unpackedBytes - zipped.bytes });
        zipped.entries += zip.files.length + zip.skipped.length;
        const folder = path.replace(ARCHIVE, "");
        for (const file of zip.files) {
          zipped.bytes += file.bytes.byteLength;
          add(`${folder}/${file.path}`, file.bytes, true);
        }
        for (const entry of zip.skipped) skip(`${folder}/${entry.path}`, entry.reason);
      } catch (error) {
        skip(path, error instanceof Error ? error.message : String(error));
      }
      return;
    }
    if (bytes.byteLength > LIMITS.fileBytes) return skip(path, `It's over ${formatBytes(LIMITS.fileBytes)}.`);
    const text = decode(bytes);
    if (text === null) return skip(path, "It isn't a text file.");
    if (texts.has(path)) return skip(path, "Another file has the same path.");
    texts.set(path, text);
  };
  for (const file of dropped) add(file.path, file.bytes, false);

  // Each skill's folder, with a "/" after it; "" for a SKILL.md dropped by itself.
  const roots = new Map<string, Map<string, string>>();
  for (const [path, text] of texts) {
    const slash = path.lastIndexOf("/");
    if (path.slice(slash + 1).toLowerCase() !== SKILL_FILE.toLowerCase()) continue;
    const folder = path.slice(0, slash + 1);
    if (roots.has(folder)) skip(path, "Its folder has another SKILL.md.");
    else roots.set(folder, new Map([[SKILL_FILE, text]]));
    texts.delete(path);
  }
  const deepestFirst = [...roots.keys()].sort((a, b) => b.length - a.length);
  // fallback is the name it gets when its front matter gives none: its folder's, or a loose file's own.
  const candidates: { path: string; fallback: string; files: Map<string, string> }[] = [];
  for (const [path, text] of texts) {
    const folder = deepestFirst.find((root) => path.startsWith(root));
    const stem = path.slice(path.lastIndexOf("/") + 1).replace(/\.md$/i, "");
    if (folder !== undefined) roots.get(folder)!.set(path.slice(folder.length), text);
    else if (!/\.md$/i.test(path)) skip(path, "It isn't in a skill: there's no SKILL.md in its folder or above it.");
    else if (NOT_SKILLS.has(stem.toLowerCase())) skip(path, `A ${stem} isn't a skill.`);
    else candidates.push({ path, fallback: stem, files: new Map([[SKILL_FILE, text]]) });
  }
  for (const [folder, files] of roots) {
    const path = folder ? folder.slice(0, -1) : SKILL_FILE;
    candidates.push({ path, fallback: folder ? path.slice(path.lastIndexOf("/") + 1) : "", files });
  }

  const skills: ImportedSkill[] = [];
  for (const { path, fallback, files } of candidates.sort((a, b) => a.path.localeCompare(b.path))) {
    const text = files.get(SKILL_FILE)!;
    const name = skillName(frontMatterValue(text, "name")) || skillName(fallback);
    if (!name) {
      skip(path, "It has no name. Give it one in its front matter, such as name: color-grading.");
      continue;
    }
    if (skills.some((skill) => skill.name === name)) {
      skip(path, `Another skill here is called ${name} too.`);
      continue;
    }
    const written = frontMatterValue(text, "description");
    const description = written.slice(0, MAX_DESCRIPTION) || firstParagraph(skillBody(text));
    if (!description && !above.has(name)) {
      skip(path, "It doesn't say what work it's for. Add a description to its front matter.");
      continue;
    }
    if (files.size > LIMITS.skillFiles) {
      skip(path, `It has over ${LIMITS.skillFiles} files.`);
      continue;
    }
    if ([...files.values()].reduce((sum, file) => sum + Buffer.byteLength(file), 0) > LIMITS.skillBytes) {
      skip(path, `It's over ${formatBytes(LIMITS.skillBytes)}.`);
      continue;
    }
    files.set(SKILL_FILE, withFrontMatter(text, name, written ? null : description || null));
    skills.push({ name, description, files });
  }
  return { skills, skipped };
}
