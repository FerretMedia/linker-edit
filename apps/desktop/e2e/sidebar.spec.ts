import { expect, test } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { openAddProject } from "./sidebar-helpers.ts";

// A project of this test's own, so the other specs running alongside don't touch its threads.
const dir = path.join(os.tmpdir(), "linker-edit-e2e", "team", "Sidebar");

test.beforeAll(() => {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "linker-edit.json"), JSON.stringify({ version: 1, name: "Sidebar", resolve: null }));
});

test("a pinned thread is settled out of the inbox, found again, and the project scope and folders work", async ({ page }) => {
  await page.goto("/");
  await openAddProject(page);
  await page.getByRole("tab", { name: "Folder" }).click();
  const field = page.getByRole("combobox", { name: "Project folder" });
  await field.fill(dir + path.sep);
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // The new project's thread: a card in the inbox that names its project, renamed so it can be found.
  const sidebar = page.getByRole("navigation", { name: "Threads" });
  const open = sidebar.getByTestId("sidebar-row-card").and(page.locator("[aria-current=page]"));
  await expect(open).toContainText("Sidebar");
  await open.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Rename thread" }).click();
  await page.getByRole("textbox", { name: "Thread title" }).fill("Trailer for the launch");
  await page.keyboard.press("Enter");
  const card = sidebar.getByTestId("sidebar-row-card").filter({ hasText: "Trailer for the launch" });
  await expect(card).toBeVisible();

  // Pinned: it stays a card, with its pin.
  await card.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Pin thread" }).click();
  await expect(card.getByRole("button", { name: "Unpin thread" })).toBeVisible();

  // The Projects menu lists only this project's threads, and All projects lists every one again.
  await page.getByRole("button", { name: "Projects" }).click();
  await page.getByRole("option", { name: /^Sidebar/ }).click();
  await expect(page.getByRole("button", { name: "Projects: Sidebar" })).toBeVisible();
  await expect(sidebar.getByTestId("sidebar-row-card")).toHaveCount(1);
  await page.getByRole("button", { name: "Projects: Sidebar" }).click();
  await page.getByRole("option", { name: "All projects" }).click();
  await expect(page.getByRole("button", { name: "Projects", exact: true })).toBeVisible();

  // Settled from its hover action: the open thread moves on, and it's off the list, counted on the closed shelf.
  await card.hover();
  await card.getByRole("button", { name: "Settle thread" }).click();
  await expect(sidebar.getByText("Trailer for the launch")).toHaveCount(0);
  await expect(sidebar.getByTestId("sidebar-settled-shelf-toggle")).toBeVisible();

  // Search finds it, and opens it; open, it shows on the closed shelf.
  await page.getByRole("combobox", { name: "Search threads" }).fill("trailer launch");
  await expect(sidebar.getByRole("option", { name: /Trailer for the launch/ })).toBeVisible();
  await page.getByRole("combobox", { name: "Search threads" }).press("Enter");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Trailer for the launch");
  const slim = sidebar.getByTestId("sidebar-row-slim").filter({ hasText: "Trailer for the launch" });
  await expect(slim).toBeVisible();
  await expect(sidebar.getByTestId("sidebar-settled-shelf-toggle")).toHaveAttribute("aria-expanded", "false");

  // Un-settled from its hover action, it's back in the inbox.
  await slim.hover();
  await slim.getByRole("button", { name: "Un-settle thread" }).click();
  await expect(card).toBeVisible();

  // Project folders, T3 Code's other layout, are a setting of this window.
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("switch", { name: "Group threads into project folders" }).click();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(sidebar.getByTestId("sidebar-project-folder").filter({ hasText: "Sidebar" })).toBeVisible();
  await expect(sidebar.getByTestId("sidebar-row-folder").filter({ hasText: "Trailer for the launch" })).toBeVisible();
});
