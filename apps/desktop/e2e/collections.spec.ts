import { expect, test, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { FAKE_LINKER_PORT, FAKE_ORGS } from "./fake-linker.ts";
import { openAddProject } from "./sidebar-helpers.ts";

// playwright.config.ts gives the server <tmp>/linker-edit-e2e, signed in at a fake Linker that lists two orgs;
// its first project is the old "workspace" folder.
const dir = path.join(os.tmpdir(), "linker-edit-e2e");

async function openBrands(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: "Brands", exact: true }).click();
  await expect(page.getByRole("navigation", { name: "Brands" })).toBeVisible();
}

const setOrgs = (orgs: unknown) => fetch(`http://127.0.0.1:${FAKE_LINKER_PORT}/fake/orgs`, { method: "PUT", body: JSON.stringify(orgs) });

test("a project goes in a series inside a sub-brand of a brand, and gets the brand's rules first", async ({ page }) => {
  await openBrands(page);
  // Each Linker org is a brand; the first one is open, with its brief.
  const nav = page.getByRole("navigation", { name: "Brands" });
  await expect(nav.getByRole("button", { name: "Ferret Media" })).toBeVisible();
  await expect(nav.getByRole("button", { name: "Studio North" })).toBeVisible();
  await expect(page.getByRole("heading", { name: "Ferret Media", level: 1 })).toBeVisible();

  await page.getByRole("textbox", { name: "Rules for the agent" }).fill("End every video on the logo.");
  await page.getByRole("button", { name: "Save rules" }).click();
  await expect(page.getByRole("button", { name: "Saved" })).toBeDisabled();

  // A sub-brand in the brand, and a series in the sub-brand picked.
  await page.getByRole("button", { name: "New sub-brand" }).click();
  await expect(page.getByRole("combobox", { name: "New sub-brand in" })).toHaveValue(/.+/);
  await page.getByRole("textbox", { name: "New sub-brand name" }).fill("Shorts");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("textbox", { name: "Sub-brand name" })).toHaveValue("Shorts");
  await page.getByRole("button", { name: "New series" }).click();
  await expect(page.getByRole("combobox", { name: "New series in" }).locator("option:checked")).toHaveText("Ferret Media / Shorts");
  await page.getByRole("textbox", { name: "New series name" }).fill("Weekly");
  await page.getByRole("button", { name: "Create" }).click();
  await expect(page.getByRole("textbox", { name: "Series name" })).toHaveValue("Weekly");

  // A sub-brand with a series in it can't be deleted.
  const tree = page.getByRole("list", { name: /^Ferret Media:/ });
  await tree.getByRole("button", { name: /^Shorts/ }).click();
  await expect(page.getByRole("button", { name: "Delete" })).toBeDisabled();

  // Add project asks for a brand, then a sub-brand or series in it.
  await page.getByRole("button", { name: "Back" }).click();
  await openAddProject(page);
  await page.getByRole("combobox", { name: "Brand", exact: true }).selectOption({ label: "Ferret Media" });
  await page.getByRole("combobox", { name: "Sub-brand or series" }).selectOption({ label: "Shorts / Weekly" });
  await page.getByRole("tab", { name: "Cloud" }).click();
  await page.getByRole("textbox", { name: "Project name, exactly as in Resolve" }).fill("Brand promo");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);

  // And project settings moves one there.
  await page.getByRole("button", { name: "Projects" }).click();
  await page.getByRole("option", { name: "workspace", exact: true }).click({ button: "right" });
  await page.getByRole("menuitem", { name: "Project settings" }).click();
  await page.getByRole("combobox", { name: "Brand, sub-brand or series" }).selectOption({ label: "Ferret Media / Shorts / Weekly" });
  await page.getByRole("button", { name: "Save" }).click();

  await openBrands(page);
  await expect(page.getByRole("list", { name: "Projects in Weekly" }).getByRole("listitem")).toHaveText([/^\s*workspace$/, /^\s*Brand promo$/]);

  // Codex reads the rules from AGENTS.md in the project's folder, brand first; Claude gets the same text.
  const agents = path.join(dir, "workspace", "AGENTS.md");
  await expect.poll(() => fs.readFileSync(agents, "utf8")).toContain(`## Brand "Ferret Media" (${path.join(dir, "collections", "ferret-media", "SKILL.md")})\n\nEnd every video on the logo.`);
  expect(JSON.parse(fs.readFileSync(path.join(dir, "workspace", "linker-edit.json"), "utf8")).collection).toMatchObject({ name: "Weekly" });
  expect(fs.readFileSync(path.join(dir, "collections", "ferret-media", "SKILL.md"), "utf8")).toContain("\n---\n\nEnd every video on the logo.");
});

test("a project with no file of its own can start a video from its brief, until its file is saved in the panel", async ({ page }) => {
  await page.goto("/");
  // Not sent here: chat.spec.ts has the one turn that may run.
  await page.getByRole("button", { name: "New thread", exact: true }).click();
  await page.getByRole("button", { name: "New video" }).click();
  const start = page.getByRole("button", { name: "Make the first cut" });
  await page.getByRole("textbox", { name: "Title" }).fill("Promo");
  await expect(start).toBeDisabled();
  await page.getByRole("button", { name: "Footage: choose a folder" }).click();
  const picker = page.getByRole("dialog", { name: "Choose: Footage" });
  await expect(picker.getByRole("combobox", { name: "Footage" })).not.toHaveValue("");
  await picker.getByRole("button", { name: /^Choose/ }).last().click();
  await expect(start).toBeEnabled();
  await page.getByRole("button", { name: "Back", exact: true }).click();
  await expect(page.getByText("What are we editing?")).toBeVisible();

  await page.getByRole("button", { name: "Project file", exact: true }).click();
  await page.getByRole("textbox", { name: "Project file" }).fill("# Promo\n\n## Brief\nThirty seconds, for the launch.");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("button", { name: "Saved", exact: true })).toBeDisabled();
  expect(fs.readFileSync(path.join(dir, "workspace", "PROJECT.md"), "utf8")).toBe("# Promo\n\n## Brief\nThirty seconds, for the launch.\n");
});

test("a brand is set up with AI in a thread of its own, and can be set to work on its own", async ({ page }) => {
  await openBrands(page);
  await page.getByRole("navigation", { name: "Brands" }).getByRole("button", { name: "Studio North" }).click();
  await expect(page.getByRole("heading", { name: "Studio North", level: 1 })).toBeVisible();

  // The setup thread opens in place of the Brands page, without the panels that need a Resolve project.
  await page.getByRole("button", { name: "Set up with AI" }).click();
  await expect(page.getByRole("heading", { name: "Teach Linker-Edit how Studio North is edited" })).toBeVisible();
  await expect(page.getByRole("navigation", { name: "Threads" }).getByTestId("sidebar-row-card").filter({ hasText: "Studio North setup" })).toBeVisible();
  await expect(page.getByRole("button", { name: "Preview" })).toHaveCount(0);

  // It can start from Resolve projects alone. Not sent here: chat.spec.ts has the one turn that may run.
  await expect(page.getByRole("button", { name: "Start", exact: true })).toBeEnabled();
  await page.getByRole("button", { name: "Finished videos: choose a folder" }).click();
  const picker = page.getByRole("dialog", { name: "Choose: Finished videos" });
  await expect(picker.getByRole("combobox", { name: "Finished videos" })).not.toHaveValue("");
  await picker.getByRole("button", { name: "Cancel" }).click();
  await expect(picker).toHaveCount(0);

  // The setup thread is found again from the brand, which can be set to work on its own. No project is put in it
  // here: chat.spec.ts expects the shared "workspace" project to start threads the usual way.
  await openBrands(page);
  await page.getByRole("navigation", { name: "Brands" }).getByRole("button", { name: "Studio North" }).click();
  await expect(page.getByRole("button", { name: "Open setup" })).toBeVisible();
  const alone = page.getByRole("switch", { name: "Works on its own" });
  await alone.click();
  await expect(alone).toBeChecked();
});

test("a brand whose org the person left on Linker is greyed and takes nothing new", async ({ page }) => {
  await setOrgs(FAKE_ORGS.filter((org) => org.id === "org_ferret"));
  try {
    await openBrands(page);
    const north = page.getByRole("navigation", { name: "Brands" }).getByRole("button", { name: "Studio North" });
    await expect(north).toHaveAttribute("title", "You're not in Studio North on Linker any more, so it takes nothing new.");
    await north.click();
    await expect(page.getByText("You're not in Studio North on Linker any more")).toBeVisible();
    await expect(page.getByRole("button", { name: "New sub-brand" })).toHaveCount(0);
    // Add project can't pick it.
    await page.getByRole("button", { name: "Back" }).click();
    await openAddProject(page);
    await expect(page.getByRole("combobox", { name: "Brand", exact: true }).getByRole("option", { name: "Studio North" })).toBeDisabled();
  } finally {
    await setOrgs(FAKE_ORGS);
  }
});
