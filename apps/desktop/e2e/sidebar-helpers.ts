import { expect, type Page } from "@playwright/test";

/** Add project: in the Projects menu once there are projects, as in T3 Code, and on the empty sidebar before. */
export async function openAddProject(page: Page) {
  const projects = page.getByRole("button", { name: /^Projects/ });
  const empty = page.getByRole("navigation", { name: "Threads" }).getByRole("button", { name: "Add project" });
  await expect(projects.or(empty).first()).toBeVisible();
  if (await projects.isVisible()) {
    await projects.click();
    await page.getByRole("option", { name: "Add project" }).click();
  } else {
    await empty.click();
  }
}
