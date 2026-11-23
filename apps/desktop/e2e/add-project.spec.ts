import { expect, test } from "@playwright/test";
import * as os from "node:os";
import * as path from "node:path";
import { openAddProject } from "./sidebar-helpers.ts";

// playwright.config.ts puts a project folder at <tmp>/linker-edit-e2e/team/Promo.
const team = path.join(os.tmpdir(), "linker-edit-e2e", "team");

test("a project folder is picked from the list instead of typed out", async ({ page }) => {
  await page.goto("/");
  await openAddProject(page);
  await page.getByRole("tab", { name: "Folder" }).click();

  const field = page.getByRole("combobox", { name: "Project folder" });
  // It starts in the home folder, written out in full.
  await expect(field).toHaveValue(new RegExp(`^${escape(os.homedir())}`));
  await expect(page.getByRole("button", { name: "Add", exact: true })).toBeDisabled();

  await field.fill(team + path.sep + "pr");
  const promo = page.getByRole("option", { name: /Promo/ });
  await expect(promo).toContainText("Linker-Edit project");
  await expect(promo).toHaveAttribute("aria-selected", "true");

  // Enter goes into the chosen folder; Enter again, with nothing chosen, adds it.
  await field.press("Enter");
  await expect(field).toHaveValue(path.join(team, "Promo") + path.sep);
  await expect(page.getByRole("option", { name: ".." })).toBeVisible();
  await expect(page.getByRole("button", { name: "Add", exact: true })).toBeEnabled();
  await field.press("Enter");

  await expect(page.getByRole("dialog")).toHaveCount(0);
  await expect(page.getByRole("navigation").getByText("Promo")).toBeVisible();
});

test("a cloud project is added by name only, and says you open it in Resolve yourself", async ({ page }) => {
  await page.goto("/");
  await openAddProject(page);
  await page.getByRole("tab", { name: "Cloud" }).click();
  await expect(page.getByRole("combobox", { name: "Media folder" })).toHaveCount(0);
  await expect(page.getByText("open Blackmagic Cloud projects in Resolve's Project Manager yourself")).toBeVisible();
});

function escape(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
