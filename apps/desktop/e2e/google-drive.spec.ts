import { expect, test, type Page } from "@playwright/test";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { E2E_DRIVE_PROJECT, FAKE_GOOGLE_USER } from "./fake-google.ts";
import { openAddProject } from "./sidebar-helpers.ts";

// playwright.config.ts puts a project folder that mirrors one on the fake Google's Drive at this path.
const reel = path.join(os.tmpdir(), "linker-edit-e2e", "team", "Launch reel");

const driveSettings = (page: Page) => page.locator("section").filter({ has: page.getByRole("heading", { name: "Google Drive" }) });

async function signIn(page: Page) {
  await page.goto("/#settings/account");
  const tab = page.context().waitForEvent("page");
  await driveSettings(page).getByRole("link", { name: "Sign in" }).click();
  await expect((await tab).getByRole("heading", { name: "Signed in to Google Drive" })).toBeVisible();
  await expect(driveSettings(page).getByText(FAKE_GOOGLE_USER.emailAddress)).toBeVisible();
}

// Against the fake Google: Sign in opens Google's consent screen in a tab, which comes back to Linker-Edit's own
// listener on 127.0.0.1, and the settings show the account.
test("signs in to Google Drive in the browser, and out again", async ({ page }) => {
  await signIn(page);
  await driveSettings(page).getByRole("button", { name: "Sign out" }).click();
  await expect(driveSettings(page).getByRole("link", { name: "Sign in" })).toBeVisible();
});

test("a project on Google Drive syncs while signed in, and says why it stopped once signed out", async ({ page }) => {
  await signIn(page);
  await page.goto("/");
  await openAddProject(page);
  await page.getByRole("tab", { name: "Folder" }).click();
  const field = page.getByRole("combobox", { name: "Project folder" });
  await field.fill(reel + path.sep);
  await expect(page.getByRole("button", { name: "Add", exact: true })).toBeEnabled();
  await field.press("Enter");

  // In the Projects menu, where the inbox lists projects.
  await page.getByRole("button", { name: /^Projects/ }).click();
  const sync = page.getByTestId("sidebar-scope-project").filter({ hasText: "Launch reel" }).getByTestId("sidebar-project-sync");
  // The fake Drive sends the clip slowly enough to see it come down.
  await expect(sync).toContainText(/Syncing with Google Drive, \d+%/);
  await expect(sync).toContainText("Up to date on Google Drive");
  expect(fs.statSync(path.join(reel, E2E_DRIVE_PROJECT.clip)).size).toBe(E2E_DRIVE_PROJECT.clipSize);

  await page.goto("/#settings/account");
  await driveSettings(page).getByRole("button", { name: "Sign out" }).click();
  await page.goto("/");
  await page.getByRole("button", { name: /^Projects/ }).click();
  await expect(sync).toContainText("Sign in to Google Drive in Settings to sync this project.");
});
