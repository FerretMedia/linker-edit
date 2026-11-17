import { expect, test } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// playwright.config.ts gives the server <tmp>/linker-edit-e2e; its first project is the old "workspace" folder.
const dir = path.join(os.tmpdir(), "linker-edit-e2e");

test("a project goes in a series inside a workspace, and gets the workspace's rules", async ({ page }) => {
  await page.goto("/#settings/workspaces");
  await expect(page.getByRole("heading", { name: "Workspaces & series" })).toBeVisible();

  await page.getByRole("button", { name: "New workspace" }).click();
  await page.getByRole("textbox", { name: "New workspace name" }).fill("Ferret Media");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("textbox", { name: "Workspace name" })).toHaveValue("Ferret Media");

  await page.getByRole("textbox", { name: "Rules for the agent" }).fill("End every video on the logo.");
  await page.getByRole("button", { name: "Save rules" }).click();
  await expect(page.getByRole("button", { name: "Saved" })).toBeDisabled();

  // A new series goes in the workspace that is picked.
  await page.getByRole("button", { name: "New series" }).click();
  await expect(page.getByRole("combobox", { name: "In workspace" })).toHaveValue(/.+/);
  await page.getByRole("textbox", { name: "New series name" }).fill("Podcast");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("textbox", { name: "Series name" })).toHaveValue("Podcast");

  // A workspace with a series in it can't be deleted.
  await page.getByRole("button", { name: /Ferret Media/ }).click();
  await expect(page.getByRole("button", { name: "Delete" })).toBeDisabled();

  await page.getByRole("button", { name: "Back" }).click();
  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("button", { name: "workspace", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Project settings" }).click();
  await page.getByRole("combobox", { name: "Workspace / series" }).selectOption({ label: "Ferret Media / Podcast" });
  await page.getByRole("button", { name: "Save" }).click();

  await page.goto("/#settings/workspaces");
  await expect(page.getByRole("button", { name: /Podcast\s*1 project/ })).toBeVisible();

  // Codex reads the rules from AGENTS.md in the project's folder; Claude gets the same text.
  const agents = path.join(dir, "workspace", "AGENTS.md");
  await expect.poll(() => fs.readFileSync(agents, "utf8")).toContain(`## Workspace "Ferret Media" (${path.join(dir, "collections", "ferret-media", "RULES.md")})\n\nEnd every video on the logo.`);
  expect(JSON.parse(fs.readFileSync(path.join(dir, "workspace", "linker-edit.json"), "utf8")).collection).toMatchObject({ name: "Podcast" });
  expect(fs.readFileSync(path.join(dir, "collections", "ferret-media", "RULES.md"), "utf8")).toBe("End every video on the logo.");
});

test("a project's own file is read and saved in the project file panel", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Project file", exact: true }).click();
  await page.getByRole("textbox", { name: "Project file" }).fill("# Promo\n\n## Brief\nThirty seconds, for the launch.");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("button", { name: "Saved", exact: true })).toBeDisabled();
  expect(fs.readFileSync(path.join(dir, "workspace", "PROJECT.md"), "utf8")).toBe("# Promo\n\n## Brief\nThirty seconds, for the launch.\n");
});

test("a workspace is set up with AI in a thread of its own, and can be set to work on its own", async ({ page }) => {
  await page.goto("/#settings/workspaces");
  await page.getByRole("button", { name: "New workspace" }).click();
  await page.getByRole("textbox", { name: "New workspace name" }).fill("Studio North");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("textbox", { name: "Workspace name" })).toHaveValue("Studio North");

  // The setup thread opens behind the settings page, without the panels that need a Resolve project.
  await page.getByRole("button", { name: "Set up with AI" }).click();
  await expect(page.getByRole("heading", { name: "Teach Linker-Edit how Studio North is edited" })).toBeVisible();
  await expect(page.getByRole("navigation").getByRole("button", { name: "Studio North setup", exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Preview" })).toHaveCount(0);

  // It starts from a folder of finished videos. Not sent here: chat.spec.ts has the one turn that may run.
  await expect(page.getByRole("button", { name: "Start", exact: true })).toBeDisabled();
  await expect(page.getByRole("combobox", { name: "Finished videos" })).toBeVisible();

  // The setup thread is found again from the workspace, which can be set to work on its own. No project is put
  // in it here: chat.spec.ts expects the shared "workspace" project to start threads the usual way.
  await page.goto("/#settings/workspaces");
  await page.getByRole("button", { name: /Studio North/ }).click();
  await expect(page.getByRole("button", { name: "Open setup" })).toBeVisible();
  const alone = page.getByRole("switch", { name: "Works on its own" });
  await alone.click();
  await expect(alone).toBeChecked();
});
