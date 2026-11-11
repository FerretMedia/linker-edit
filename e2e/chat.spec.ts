import { expect, test } from "@playwright/test";

test("a turn streams in, waits for each change to be approved, and finishes", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("textbox").fill("Check whether Resolve is reachable");
  await page.getByRole("button", { name: "Send" }).click();

  const approval = page.getByText(/^Allow (Resolve|tool):/).locator("..");
  // A script through Resolve's own run_script always asks, and shows the script itself.
  await expect(approval).toHaveText(/Allow Resolve:\s*run_script/);
  await expect(page.locator("pre").filter({ hasText: "result = resolve.GetVersionString()" }).first()).toBeVisible();
  await page.getByRole("button", { name: "Allow", exact: true }).click();

  await expect(approval).toHaveText(/Allow tool:\s*Write/);
  await page.getByRole("button", { name: "Deny", exact: true }).click();

  await expect(page.getByText("DaVinci Resolve is reachable.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  await expect(approval).toHaveCount(0);

  // The recorded turn ends with about 45K of Claude's 1M-token window in use.
  await expect(page.getByRole("button", { name: "Context: 4% used" })).toBeVisible();

  // A new thread starts empty; the first one is still there to go back to.
  const sidebar = page.getByRole("navigation");
  await sidebar.getByRole("button", { expanded: true }).first().hover();
  await sidebar.getByRole("button", { name: /^New thread in / }).click();
  await expect(page.getByText("What are we editing?")).toBeVisible();
  await sidebar.getByRole("button", { name: /Check whether Resolve is reachable/ }).click();
  await expect(page.getByText("DaVinci Resolve is reachable.")).toBeVisible();
});
