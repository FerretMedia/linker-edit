import { expect, test } from "@playwright/test";
import { FAKE_GOOGLE_USER } from "./fake-google.ts";

// Against the fake Google: Sign in opens Google's consent screen in a tab, which comes back to Linker-Edit's own
// listener on 127.0.0.1, and the settings show the account.
test("signs in to Google Drive in the browser, and out again", async ({ page, context }) => {
  await page.goto("/#settings/account");
  const drive = page.locator("section").filter({ has: page.getByRole("heading", { name: "Google Drive" }) });

  const tab = context.waitForEvent("page");
  await drive.getByRole("link", { name: "Sign in" }).click();
  await expect((await tab).getByRole("heading", { name: "Signed in to Google Drive" })).toBeVisible();
  await expect(drive.getByText(FAKE_GOOGLE_USER.emailAddress)).toBeVisible();

  await drive.getByRole("button", { name: "Sign out" }).click();
  await expect(drive.getByRole("link", { name: "Sign in" })).toBeVisible();
});
