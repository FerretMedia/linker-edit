import { expect, test } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";

// A project of this test's own, so the other specs running alongside don't touch its threads.
const dir = path.join(os.tmpdir(), "linker-edit-e2e", "team", "Sidebar");

test.beforeAll(() => {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, "linker-edit.json"), JSON.stringify({ version: 1, name: "Sidebar", resolve: null }));
});

test("a pinned thread is settled out of the list, and search still finds it", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Add project" }).first().click();
  await page.getByRole("tab", { name: "Folder" }).click();
  const field = page.getByRole("combobox", { name: "Project folder" });
  await field.fill(dir + path.sep);
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  const sidebar = page.getByRole("navigation", { name: "Threads" });
  const folder = sidebar.getByTestId("sidebar-project-folder").filter({ hasText: "Sidebar" });
  await expect(folder).toBeVisible();

  // The new project's thread, renamed so it can be found.
  const row = sidebar.getByTestId("sidebar-row-folder").and(page.locator("[aria-current=page]"));
  await row.click({ button: "right" });
  await page.getByRole("menuitem", { name: "Rename thread" }).click();
  await page.getByRole("textbox", { name: "Thread title" }).fill("Trailer for the launch");
  await page.keyboard.press("Enter");
  await expect(sidebar.getByTestId("sidebar-row-folder").filter({ hasText: "Trailer for the launch" })).toBeVisible();

  // Pinned: it leaves the project folder for a card on top.
  await sidebar.getByTestId("sidebar-row-folder").filter({ hasText: "Trailer for the launch" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Pin thread" }).click();
  const card = sidebar.getByTestId("sidebar-row-card").filter({ hasText: "Trailer for the launch" });
  await expect(card).toBeVisible();
  await expect(card).toContainText("Sidebar");

  // Settled: off the list, counted on the closed Settled shelf.
  await card.hover();
  await card.getByRole("button", { name: "Settle thread" }).click();
  await expect(sidebar.getByText("Trailer for the launch")).toHaveCount(0);
  await expect(sidebar.getByTestId("sidebar-settled-shelf-toggle")).toBeVisible();

  // Search finds it, and opens it.
  await page.getByRole("combobox", { name: "Search threads" }).fill("trailer launch");
  const result = sidebar.getByRole("option", { name: /Trailer for the launch/ });
  await expect(result).toBeVisible();
  await page.getByRole("combobox", { name: "Search threads" }).press("Enter");
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Trailer for the launch");

  // Un-settled from its menu, it goes back to its project.
  await sidebar.getByTestId("sidebar-settled-shelf-toggle").click();
  await sidebar.getByTestId("sidebar-row-slim").filter({ hasText: "Trailer for the launch" }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Un-settle thread" }).click();
  await expect(sidebar.getByTestId("sidebar-row-folder").filter({ hasText: "Trailer for the launch" })).toBeVisible();
});
