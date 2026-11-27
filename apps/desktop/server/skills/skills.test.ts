import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { BRIEF_TOOLS } from "../collections/tools.ts";
import { APP_ROOT } from "../config.ts";
import { LIBRARY_TOOLS } from "../library/tools.ts";
import { RESOLVE_TOOLS } from "../resolve/tools.ts";
import { AUTONOMOUS_RULES } from "../rules.ts";
import { STUDY_TOOLS } from "../study/study.ts";
import type { ImportedSkill } from "./import.ts";
import { listSkills, READ_SKILL_TOOL, readSkill, removeSkill, savedSkills, skillsSection, writeSkills, type SkillRoot } from "./skills.ts";

let dir: string;
let roots: SkillRoot[];

function write(root: string, file: string, text: string) {
  const full = path.join(dir, root, file);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  fs.writeFileSync(full, text);
}

beforeEach(() => {
  dir = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-skills-"));
  roots = [
    { label: "Linker-Edit", dir: path.join(dir, "shipped") },
    { label: 'Workspace "Ferret"', dir: path.join(dir, "workspace") },
    { label: 'Series "Reels"', dir: path.join(dir, "series") },
  ];
  write("shipped", "color/SKILL.md", '---\nname: color\ndescription: "Grading \\"footage\\"."\n---\n\nBalance first.\n');
  write("shipped", "color/skin.md", "Skin on the line.");
  write("shipped", "sound/SKILL.md", "---\r\nname: sound\r\ndescription: Music and levels.\r\n---\r\n\r\nMusic under speech.\r\n");
});

describe("imported skills", () => {
  const own = () => path.join(dir, "series");
  const skill = (name: string, files: Record<string, string>): ImportedSkill => ({ name, description: "", files: new Map(Object.entries(files)) });

  it("are written as skills the agent reads, and replace a skill of their name whole", () => {
    expect(writeSkills(own(), [skill("podcast", { "SKILL.md": "---\nname: podcast\ndescription: Clips.\n---\n\nOne idea.", "references/hooks.md": "First line." })])).toEqual({
      imported: ["podcast"],
      replaced: [],
      skipped: [],
    });
    expect(savedSkills(own())).toEqual([{ name: "podcast", description: "Clips.", file: path.join(own(), "podcast", "SKILL.md") }]);
    expect(readSkill(roots, "podcast", "references/hooks.md")).toBe("First line.");

    expect(writeSkills(own(), [skill("podcast", { "SKILL.md": "---\nname: podcast\ndescription: Shorter clips.\n---\n\nTwo ideas." })])).toEqual({
      imported: ["podcast"],
      replaced: ["podcast"],
      skipped: [],
    });
    expect(readSkill(roots, "podcast")).toMatch(/Two ideas\.$/);
    expect(() => readSkill(roots, "podcast", "references/hooks.md")).toThrow(/no file/);
    expect(fs.readdirSync(own())).toEqual(["podcast"]);
  });

  it("leave the skill they would replace as it was when writing fails, and nothing half written", () => {
    write("series", "color/SKILL.md", "Warm.");
    write("series", ".importing-stale/SKILL.md", "From an import that stopped half way.");
    // A file where a folder has to go.
    const result = writeSkills(own(), [skill("color", { "SKILL.md": "Cold.", notes: "x", "notes/more.md": "y" }), skill("sound", { "SKILL.md": "Loud." })]);
    expect(result).toMatchObject({ imported: ["sound"], replaced: [], skipped: [{ path: "color", reason: expect.stringMatching(/^Couldn't write it: /) }] });
    expect(readSkill(roots, "color")).toMatch(/Warm\.$/);
    expect(fs.readdirSync(own()).sort()).toEqual(["color", "sound"]);
  });

  it("are removed by name only", () => {
    write("series", "color/SKILL.md", "Warm.");
    expect(removeSkill(own(), "../shipped")).toBeNull();
    expect(listSkills(roots.slice(0, 1))).toHaveLength(2);
    expect(removeSkill(own(), "color")).toBeNull();
    expect(savedSkills(own())).toEqual([]);
  });
});

describe("skills", () => {
  it("lists every skill once, with where each part comes from and the most specific description", () => {
    write("series", "color/SKILL.md", "---\nname: color\ndescription: The Reels grade.\n---\n\nWarm.");
    write("workspace", "color/SKILL.md", "Teal shadows.");
    write("workspace", "Not A Skill/SKILL.md", "No.");
    write("workspace", "empty/notes.md", "No SKILL.md.");
    expect(skillsSection(listSkills(roots)).split("\n").filter((line) => line.startsWith("- "))).toEqual([
      '- color: The Reels grade. (Linker-Edit, Workspace "Ferret", Series "Reels")',
      "- sound: Music and levels. (Linker-Edit)",
    ]);
    expect(listSkills(roots.slice(0, 1))[0]!.description).toBe('Grading "footage".');
    for (const [written, read] of [
      ["description: >\n  Grading\n  footage.", "Grading footage."],
      ["# mine\ndescription: 'Don''t'", "Don't"],
      ["description: >-\n  One: two.\nother: x", "One: two."],
    ] as const) {
      write("workspace", "color/SKILL.md", `---\n${written}\n---\n\nTeal.`);
      expect(listSkills(roots.slice(0, 2))[0]!.description).toBe(read);
      expect(readSkill(roots.slice(0, 2), "color")).toMatch(/SKILL\.md\)\n\nTeal\.$/);
    }
    expect(skillsSection(listSkills([{ label: "None", dir: path.join(dir, "missing") }]))).toBe("");
  });

  it("gives the parts most general first, without their front matter", () => {
    const color = path.join(dir, "shipped", "color", "SKILL.md");
    expect(readSkill(roots, "color")).toBe(`## Linker-Edit (${color})\n\nBalance first.`);
    write("series", "color/SKILL.md", "---\nname: color\n---\n\nWarm.");
    expect(readSkill(roots, "color")).toBe(
      `Later parts add to earlier ones and win where they disagree.\n\n## Linker-Edit (${color})\n\nBalance first.\n\n## Series "Reels" (${path.join(dir, "series", "color", "SKILL.md")})\n\nWarm.`,
    );
  });

  it("gives a file of a skill from the most specific place that has it, and nothing outside the skill", () => {
    write("series", "color/SKILL.md", "Warm.");
    expect(readSkill(roots, "color", "skin.md")).toBe("Skin on the line.");
    write("series", "color/skin.md", "Warmer skin.");
    expect(readSkill(roots, "color", "skin.md")).toBe("Warmer skin.");
    // Also where only the file is changed, and not the skill.
    write("workspace", "sound/rock.md", "Loud.");
    expect(readSkill(roots, "sound", "rock.md")).toBe("Loud.");
    expect(readSkill(roots, "color", "")).toBe(readSkill(roots, "color"));
    // Asked for as if it were a skill.
    expect(readSkill(roots, "skin")).toBe("Warmer skin.");
    expect(() => readSkill(roots, "../color/skin")).toThrow(/no skill/);
    write("shipped", "secret.md", "No.");
    write("shipped", "color/.git/config", "No.");
    for (const file of ["../secret.md", "..\\secret.md", "references/../../secret.md", path.join(dir, "shipped", "secret.md"), "/secret.md", "C:secret.md", ".git/config"]) {
      expect(() => readSkill(roots, "color", file)).toThrow(/isn't a path in the skill's folder/);
    }
    expect(() => readSkill(roots, "color", "gone.md")).toThrow(/no file/);
    expect(() => readSkill(roots, "color", "references")).toThrow(/no file/);
  });

  it("gives a file in a folder of a skill, by its path or its name alone, and any text file", () => {
    write("shipped", "color/references/print.md", "Matte paper.");
    write("series", "color/scripts/levels.py", "print('-14 LUFS')");
    write("series", "color/scripts/logo.png", "\x89PNG\0\0");
    expect(readSkill(roots, "color", "references/print.md")).toBe("Matte paper.");
    expect(readSkill(roots, "color", "print.md")).toBe("Matte paper.");
    expect(readSkill(roots, "print")).toBe("Matte paper.");
    expect(readSkill(roots, "color", "scripts/levels.py")).toBe("print('-14 LUFS')");
    expect(() => readSkill(roots, "color", "scripts/logo.png")).toThrow(/isn't a text file/);
    expect(() => readSkill(roots, "../color")).toThrow('There is no skill "../color". There are: color, sound.');
  });

  it("cuts off a skill that is too long", () => {
    write("shipped", "color/SKILL.md", "x".repeat(30_000));
    expect(readSkill(roots, "color").length).toBeLessThan(20_500);
  });
});

describe("the skills Linker-Edit ships", () => {
  const shipped = path.join(APP_ROOT, "skills");
  const skills = listSkills([{ label: "Linker-Edit", dir: shipped }]);
  const files = skills.flatMap(({ name }) => fs.readdirSync(path.join(shipped, name)).map((file) => [name, file] as const));
  const tools = [...RESOLVE_TOOLS, ...LIBRARY_TOOLS, ...STUDY_TOOLS, ...BRIEF_TOOLS, READ_SKILL_TOOL];
  // transcribe is linkerMcp.ts's own; list_luts is on Resolve's MCP server.
  const words = new Set(["transcribe", "list_luts", ...tools.flatMap((tool) => [tool.name, ...Object.keys(tool.inputSchema.properties ?? {})])]);

  it("have video-editing, which the rules for working alone start from, and say when to read them", () => {
    expect(AUTONOMOUS_RULES).toContain("video-editing skill");
    expect(skills.map(({ name }) => name)).toContain("video-editing");
    for (const skill of skills) expect(skill.description.length, skill.name).toBeGreaterThan(40);
  });

  it.each(files)("%s/%s can be read, is short, and names only tools, skills and files that exist", (name, file) => {
    const text = file === "SKILL.md" ? readSkill([{ label: "Linker-Edit", dir: shipped }], name) : readSkill([{ label: "Linker-Edit", dir: shipped }], name, file);
    expect(text.length).toBeLessThan(6_000);
    const named = [...text.matchAll(/`([a-z]+(?:[_-][a-z]+)*(?:\.md)?)`/g)].map((match) => match[1]!);
    const exists = (word: string) =>
      word.endsWith(".md") ? fs.existsSync(path.join(shipped, name, word)) : word.includes("-") ? skills.some((skill) => skill.name === word) : !word.includes("_") || words.has(word);
    expect(named.filter((word) => !exists(word))).toEqual([]);
  });
});
