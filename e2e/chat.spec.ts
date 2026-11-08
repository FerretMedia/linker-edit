import { expect, test } from "@playwright/test";

test("a turn streams in, waits for each change to be approved, and finishes", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("textbox").fill("Check whether Resolve is reachable");
  await page.getByRole("button", { name: "Send" }).click();

  const approval = page.getByText(/^Allow (Resolve|tool):/).locator("..");
  await expect(approval).toHaveText(/Allow Resolve:\s*resolve_control\.runtime_mode/);
  await page.getByRole("button", { name: "Allow", exact: true }).click();

  await expect(approval).toHaveText(/Allow tool:\s*Write/);
  await page.getByRole("button", { name: "Deny", exact: true }).click();

  await expect(page.getByText("DaVinci Resolve is reachable.")).toBeVisible();
  await expect(page.getByRole("button", { name: "Send" })).toBeVisible();
  await expect(approval).toHaveCount(0);

  // The recorded turn ends with about 45K of Claude's 1M-token window in use.
  await expect(page.getByRole("button", { name: "Context: 4% used" })).toBeVisible();
});
