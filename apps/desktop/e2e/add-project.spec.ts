import { expect, test } from "@playwright/test";
import * as os from "node:os";
import * as path from "node:path";

// playwright.config.ts puts a project folder at <tmp>/linker-edit-e2e/team/Promo.
const team = path.join(os.tmpdir(), "linker-edit-e2e", "team");

test("a project folder is picked from the list instead of typed out", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Add project" }).click();
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

test("the cloud media folder only fills in once it's used", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Add project" }).click();
  await page.getByRole("tab", { name: "Cloud" }).click();
  const media = page.getByRole("combobox", { name: "Media folder" });
  await expect(media).toHaveValue("");
  await expect(page.getByRole("listbox", { name: "Folders" })).toHaveCount(0);
  await media.focus();
  await expect(media).toHaveValue(new RegExp(`^${escape(os.homedir())}`));
  await expect(page.getByRole("listbox", { name: "Folders" })).toBeVisible();
});

function escape(value: string) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
