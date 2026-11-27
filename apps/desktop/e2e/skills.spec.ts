import { expect, test, type Locator } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { zip } from "../server/skills/testing.ts";

// playwright.config.ts gives the server <tmp>/linker-edit-e2e; your own skills are in its skills folder.
const dir = path.join(os.tmpdir(), "linker-edit-e2e");
const skill = (name: string, description: string) => `---\nname: ${name}\ndescription: ${description}\n---\n\nHow it's done.\n`;

/**
 * Drops files on an element as Chromium hands a dropped folder to the page: entries to walk, not a list of files. A
 * synthetic DataTransfer can't hold folders, so the entries are made here. Paths ending in .zip get bytes.
 */
async function drop(target: Locator, files: Record<string, string | number[]>, types = ["dragenter", "dragover", "drop"]) {
  await target.evaluate(
    (element, { files, types }) => {
      interface Node {
        name: string;
        fullPath: string;
        isFile: boolean;
        isDirectory: boolean;
        children: Node[];
        content?: string | number[];
      }
      const top: Node[] = [];
      for (const [file, content] of Object.entries(files)) {
        let level = top;
        const parts = file.split("/");
        parts.forEach((name, index) => {
          const fullPath = `/${parts.slice(0, index + 1).join("/")}`;
          let node = level.find((entry) => entry.name === name);
          if (!node) {
            node = { name, fullPath, isFile: index === parts.length - 1, isDirectory: index < parts.length - 1, children: [], content };
            level.push(node);
          }
          level = node.children;
        });
      }
      const entry = (node: Node): unknown =>
        node.isFile
          ? { ...node, file: (done: (file: File) => void) => done(new File([typeof node.content === "string" ? node.content : new Uint8Array(node.content!)], node.name)) }
          : {
              ...node,
              createReader: () => {
                let read = false;
                return { readEntries: (done: (entries: unknown[]) => void) => done(read ? [] : ((read = true), node.children.map(entry))) };
              },
            };
      const dataTransfer = { types: ["Files"], items: top.map((node) => ({ webkitGetAsEntry: () => entry(node) })), dropEffect: "none" };
      for (const type of types) {
        const event = new DragEvent(type, { bubbles: true, cancelable: true });
        Object.defineProperty(event, "dataTransfer", { value: dataTransfer });
        element.dispatchEvent(event);
      }
    },
    { files, types },
  );
}

test("a folder of skills and a zip dropped on a brand's Skills card are listed, with what was left out", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Brands", exact: true }).click();
  await expect(page.getByRole("heading", { name: "Ferret Media", level: 1 })).toBeVisible();
  const card = page.getByRole("region", { name: "Skills" });

  await drop(card, { "x.md": "" }, ["dragenter"]);
  await expect(card.getByText("Drop to import")).toBeVisible();
  await drop(card, { "x.md": "" }, ["dragleave"]);
  await expect(card.getByText("Drop to import")).toHaveCount(0);

  const exported = zip({ "hooks/SKILL.md": skill("hooks", "The first three seconds."), "__MACOSX/hooks/._SKILL.md": "x" });
  await drop(card, {
    "brand-skills/podcast/SKILL.md": skill("podcast", "Cutting a podcast into clips."),
    "brand-skills/podcast/references/interview.md": "Two cameras.",
    "brand-skills/podcast/.git/HEAD": "ref: main",
    // Adds to Linker-Edit's color-grading, so it needs no description.
    "brand-skills/color-grading/SKILL.md": "# Warm highlights",
    "brand-skills/notes.txt": "Not a skill.",
    "hooks.zip": [...exported],
  });

  await expect(card.getByRole("status")).toContainText("Imported 3 skills, skipped 3.");
  // The fake Linker has the person as an admin of Ferret Media, so they went to Linker too.
  await expect(card.getByText("Synced with Linker")).toBeVisible();
  const names = card.getByRole("listitem").filter({ has: page.getByRole("button", { name: /^Remove the skill/ }) });
  await expect(names).toHaveText([/^\s*color-grading\s*Adds to the one every project gets/, /^\s*hooks\s*The first three seconds\./, /^\s*podcast\s*Cutting a podcast/]);
  await card.getByText("What was skipped").click();
  await expect(card.getByRole("status").getByRole("listitem")).toHaveText([
    /brand-skills\/podcast\/\.git · It's hidden\./,
    /hooks\/__MACOSX · It's hidden\./,
    /brand-skills\/notes\.txt · It isn't in a skill/,
  ]);
  const skills = path.join(dir, "collections", "ferret-media", "skills");
  expect(fs.readFileSync(path.join(skills, "podcast", "references", "interview.md"), "utf8")).toBe("Two cameras.");
  expect(fs.existsSync(path.join(skills, "podcast", ".git"))).toBe(false);

  // The same name again replaces the folder whole.
  await drop(card, { "podcast/SKILL.md": skill("podcast", "Shorter clips.") });
  await expect(card.getByRole("status")).toContainText("Imported 1 skill, replaced 1.");
  await expect(names.filter({ hasText: "podcast" })).toContainText("Shorter clips.");
  expect(fs.existsSync(path.join(skills, "podcast", "references"))).toBe(false);

  page.once("dialog", (dialog) => void dialog.accept());
  await card.getByRole("button", { name: "Remove the skill podcast" }).click();
  await expect(card.getByRole("button", { name: "Remove the skill podcast" })).toHaveCount(0);
  expect(fs.existsSync(path.join(skills, "podcast"))).toBe(false);
});

test("your own skills are imported from a folder in Settings, for every project", async ({ page }) => {
  const picked = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-e2e-own-"));
  fs.mkdirSync(path.join(picked, "mine", "pacing"), { recursive: true });
  fs.writeFileSync(path.join(picked, "mine", "pacing", "SKILL.md"), skill("pacing", "How fast to cut."));
  fs.writeFileSync(path.join(picked, "mine", "thumbnails.md"), "# Thumbnails\n\nOne face, three words.");

  await page.goto("/#settings/skills");
  await expect(page.getByRole("heading", { name: "Your own skills" })).toBeVisible();
  const card = page.getByRole("region", { name: "Skills" });
  await page.getByLabel("Import a folder of skills").setInputFiles(path.join(picked, "mine"));
  await expect(card.getByRole("status")).toContainText("Imported 2 skills.");
  await expect(card.getByText("Synced with Linker")).toBeVisible();
  await expect(card.getByRole("button", { name: /^Remove the skill/ })).toHaveCount(2);
  await expect(card.getByText("One face, three words.")).toBeVisible();
  expect(fs.readFileSync(path.join(dir, "skills", "thumbnails", "SKILL.md"), "utf8")).toBe(
    "---\nname: thumbnails\ndescription: 'One face, three words.'\n---\n\n# Thumbnails\n\nOne face, three words.",
  );
});
