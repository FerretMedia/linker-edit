import { crc32, deflateRawSync } from "node:zlib";
import { describe, expect, it } from "vitest";
import { parseSkillImport, type DroppedFile } from "./import.ts";
import { unzip } from "./zip.ts";

const text = (path: string, body: string): DroppedFile => ({ path, bytes: new TextEncoder().encode(body) });
const skill = (name: string, description: string, body = "Do it well.") => `---\nname: ${name}\ndescription: ${description}\n---\n\n${body}\n`;

/** A zip as zip tools write one; `stored` leaves the files uncompressed. */
function zip(files: Record<string, string | Uint8Array>, options: { stored?: boolean; claimedSize?: number } = {}): Uint8Array {
  const locals: Buffer[] = [];
  const entries: Buffer[] = [];
  let offset = 0;
  for (const [path, content] of Object.entries(files)) {
    const data = Buffer.from(content);
    const packed = options.stored ? data : deflateRawSync(data);
    const name = Buffer.from(path);
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(options.stored ? 0 : 8, 8);
    local.writeUInt32LE(crc32(data), 14);
    local.writeUInt32LE(packed.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(name.length, 26);
    const entry = Buffer.alloc(46);
    entry.writeUInt32LE(0x02014b50, 0);
    entry.writeUInt16LE(options.stored ? 0 : 8, 10);
    entry.writeUInt32LE(crc32(data), 16);
    entry.writeUInt32LE(packed.length, 20);
    entry.writeUInt32LE(options.claimedSize ?? data.length, 24);
    entry.writeUInt16LE(name.length, 28);
    entry.writeUInt32LE(offset, 42);
    locals.push(local, name, packed);
    entries.push(entry, name);
    offset += local.length + name.length + packed.length;
  }
  const directory = Buffer.concat(entries);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(Object.keys(files).length, 8);
  end.writeUInt16LE(Object.keys(files).length, 10);
  end.writeUInt32LE(directory.length, 12);
  end.writeUInt32LE(offset, 16);
  return new Uint8Array(Buffer.concat([...locals, directory, end]));
}

/** What a user sees of an import: each skill's name, description and files, and each skip. */
function imported(files: DroppedFile[], above: string[] = []) {
  const { skills, skipped } = parseSkillImport(files, new Set(above));
  return {
    skills: skills.map((found) => ({ name: found.name, description: found.description, files: [...found.files.keys()].sort() })),
    skipped: skipped.map((entry) => `${entry.path}: ${entry.reason}`),
  };
}

describe("importing skills", () => {
  it.each<[string, DroppedFile[], ReturnType<typeof imported>]>([
    [
      "one SKILL.md, named by its front matter",
      [text("SKILL.md", skill("Color Grading", "Grading footage."))],
      { skills: [{ name: "color-grading", description: "Grading footage.", files: ["SKILL.md"] }], skipped: [] },
    ],
    [
      "a skill's folder, with its nested files, named by the folder",
      [
        text("podcast/skill.md", "---\ndescription: Cutting podcasts.\n---\n\nRead references/interview.md."),
        text("podcast/references/interview.md", "Two cameras."),
        text("podcast/scripts/levels.py", "print('ok')"),
      ],
      { skills: [{ name: "podcast", description: "Cutting podcasts.", files: ["SKILL.md", "references/interview.md", "scripts/levels.py"] }], skipped: [] },
    ],
    [
      "a folder of skills, one inside another's folder, and loose notes",
      [
        text("mine/README.md", "# My skills"),
        text("mine/color/SKILL.md", skill("color", "Grading.")),
        text("mine/color/skin.md", "Skin on the line."),
        text("mine/color/print/SKILL.md", skill("print", "Grading for print.")),
        text("mine/color/print/paper.md", "Matte."),
        text("mine/hooks.md", "# Hooks\n\nThe first three seconds sell the video.\n\nMore."),
        text("mine/notes.txt", "Not a skill."),
      ],
      {
        skills: [
          { name: "color", description: "Grading.", files: ["SKILL.md", "skin.md"] },
          { name: "print", description: "Grading for print.", files: ["SKILL.md", "paper.md"] },
          { name: "hooks", description: "The first three seconds sell the video.", files: ["SKILL.md"] },
        ],
        skipped: ["mine/README.md: A README isn't a skill.", "mine/notes.txt: It isn't in a skill: there's no SKILL.md in its folder or above it."],
      },
    ],
    [
      "hidden files, binaries, big files and bad paths left out with a reason",
      [
        text("color/SKILL.md", skill("color", "Grading.")),
        text("color/.git/config", "[core]"),
        text("color/.git/HEAD", "ref"),
        text("__MACOSX/color/._SKILL.md", "x"),
        { path: "color/logo.png", bytes: new Uint8Array([0x89, 0x50, 0x4e, 0x47, 0, 0]) },
        { path: "color/latin1.md", bytes: new Uint8Array([0x63, 0x61, 0x66, 0xe9]) },
        text("color/huge.md", "x".repeat(300 * 1024)),
        text("../outside.md", "x"),
      ],
      {
        skills: [{ name: "color", description: "Grading.", files: ["SKILL.md"] }],
        skipped: [
          "color/.git: It's hidden.",
          "__MACOSX: It's hidden.",
          "color/logo.png: It isn't a text file.",
          "color/latin1.md: It isn't a text file.",
          "color/huge.md: It's over 256 KB.",
          "../outside.md: Its path points outside what was dropped.",
        ],
      },
    ],
    [
      "a skill with no description, unless it adds to one above it",
      [text("a/SKILL.md", "# Only a heading"), text("sound/SKILL.md", "---\nname: sound\n---\n\n# Louder")],
      { skills: [], skipped: ["a: It doesn't say what work it's for. Add a description to its front matter.", "sound: It doesn't say what work it's for. Add a description to its front matter."] },
    ],
    [
      "two skills of one name, and one with no name",
      [text("a/SKILL.md", skill("color", "One.")), text("b/SKILL.md", skill("color", "Two.")), text("SKILL.md", "---\ndescription: Nameless.\n---\n")],
      {
        skills: [{ name: "color", description: "One.", files: ["SKILL.md"] }],
        skipped: ["b: Another skill here is called color too.", "SKILL.md: It has no name. Give it one in its front matter, such as name: color-grading."],
      },
    ],
    [
      "a skill with too many files",
      [text("big/SKILL.md", skill("big", "Big.")), ...Array.from({ length: 200 }, (_, index) => text(`big/refs/${index}.md`, "x"))],
      { skills: [], skipped: ["big: It has over 200 files."] },
    ],
  ])("%s", (_, files, expected) => {
    expect(imported(files)).toEqual(expected);
  });

  it("lets a skill without a description add to one of its name above it", () => {
    expect(imported([text("sound/SKILL.md", "---\nname: sound\n---\n\n# Louder")], ["sound"]).skills).toEqual([{ name: "sound", description: "", files: ["SKILL.md"] }]);
  });

  it("writes the name and description into SKILL.md, and keeps its other front matter and body", () => {
    const [found] = parseSkillImport([
      text("x/My Skill.md", "---\nlicense: MIT\nname: \"My Skill\"\nmetadata:\n  author: Mila\n---\n\n# Title\n\nDon't cut\nmid-word.\n"),
      text("y/SKILL.md", "---\nname: y\ndescription: >\n  Kept as\n  written.\nlicense: MIT\n---\n\nBody.\n"),
    ]).skills;
    expect(found!.files.get("SKILL.md")).toBe("---\nname: my-skill\ndescription: 'Don''t cut mid-word.'\nlicense: MIT\nmetadata:\n  author: Mila\n---\n\n# Title\n\nDon't cut\nmid-word.\n");
    const kept = parseSkillImport([text("y/SKILL.md", "---\nname: Why\ndescription: >\n  Kept as\n  written.\nlicense: MIT\n---\n\nBody.\n")]).skills[0]!;
    expect(kept.description).toBe("Kept as written.");
    expect(kept.files.get("SKILL.md")).toBe("---\nname: why\ndescription: >\n  Kept as\n  written.\nlicense: MIT\n---\n\nBody.\n");
  });

  it("unpacks zips and .skill exports into a folder named after them", () => {
    const files = [
      text("loose/SKILL.md", skill("loose", "Loose.")),
      { path: "Brand Pack.zip", bytes: zip({ "color/SKILL.md": skill("color", "Grading."), "color/references/skin.md": "Skin.", "__MACOSX/color/._SKILL.md": "x", "titles/": "" }) },
      { path: "sound.skill", bytes: zip({ "SKILL.md": "---\ndescription: Music.\n---\n\nUnder speech." }, { stored: true }) },
      { path: "broken.zip", bytes: new Uint8Array([1, 2, 3]) },
    ];
    expect(imported(files)).toEqual({
      skills: [
        { name: "color", description: "Grading.", files: ["SKILL.md", "references/skin.md"] },
        { name: "loose", description: "Loose.", files: ["SKILL.md"] },
        { name: "sound", description: "Music.", files: ["SKILL.md"] },
      ],
      skipped: ["Brand Pack/__MACOSX: It's hidden.", "broken.zip: It isn't a zip file."],
    });
  });

  it("refuses a zip that unpacks to too much, whatever sizes it claims", () => {
    const bomb = zip({ "a/SKILL.md": "x".repeat(40 * 1024 * 1024) }, { claimedSize: 10 });
    expect(bomb.byteLength).toBeLessThan(100 * 1024);
    expect(imported([{ path: "bomb.zip", bytes: bomb }]).skipped).toEqual([expect.stringMatching(/^bomb\.zip: It's damaged, or unpacks to more than 32 MB\.$/)]);
    expect(() => unzip(zip({ a: "1", b: "2", c: "3" }), { entries: 2, bytes: 100 })).toThrow("more than 2 files");
    expect(() => unzip(zip({ a: "x".repeat(200) }), { entries: 2, bytes: 100 })).toThrow("more than");
  });
});
