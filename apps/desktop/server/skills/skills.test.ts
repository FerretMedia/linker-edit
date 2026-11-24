import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { beforeEach, describe, expect, it } from "vitest";
import { listSkills, readSkill, skillsSection, type SkillRoot } from "./skills.ts";

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
    for (const file of ["../secret.md", "..\\secret.md", path.join(dir, "shipped", "secret.md"), "SKILL.md.bak"]) {
      expect(() => readSkill(roots, "color", file)).toThrow(/isn't a file name/);
    }
    expect(() => readSkill(roots, "color", "gone.md")).toThrow(/no file/);
    expect(() => readSkill(roots, "../color")).toThrow('There is no skill "../color". There are: color, sound.');
  });

  it("cuts off a skill that is too long", () => {
    write("shipped", "color/SKILL.md", "x".repeat(30_000));
    expect(readSkill(roots, "color").length).toBeLessThan(20_500);
  });
});
