import { expect, test, type Page } from "@playwright/test";
import type { AppState } from "../shared/protocol.ts";
import { fakeMachine, project, templateState, thread } from "./fake-machine.ts";
import { openAddProject } from "./sidebar-helpers.ts";

// This computer and an edit rig, both stand-ins, and a laptop that's offline with the projects it had last time.
let template: AppState;

test.beforeAll(async ({}, testInfo) => {
  template = await templateState(String(testInfo.project.use.baseURL));
});

async function threeMachines(page: Page) {
  await page.route("**/api/machines", (route) =>
    route.fulfill({
      json: {
        machines: [
          { id: "this", name: "Studio-PC", version: template.version, via: "this", online: true, base: "" },
          { id: "rig", name: "Edit-Rig", version: template.version, via: "linker", online: true, base: "/machines/rig" },
          { id: "laptop", name: "Laptop", version: "", via: "tailscale", online: false, base: "/machines/laptop" },
        ],
      },
    }),
  );
  const laptop = { name: "Laptop", projects: [project("old", "Old trailer", [thread("t9", "Trailer cut", { busy: true })])] };
  await page.addInitScript((cached) => localStorage.setItem("linker-edit:machines", JSON.stringify({ laptop: cached })), laptop);
  // The same thread id on two machines: only the machine tells them apart.
  const here = await fakeMachine(page, "/ws", template, "Studio-PC", [
    project("promo", "Spring promo", [thread("t1", "Cut the teaser", { busy: true }), thread("t2", "Captions", { lastSentAt: null })]),
  ]);
  const rig = await fakeMachine(page, "/machines/rig/ws", template, "Edit-Rig", [
    project("wedding", "Wedding film", [thread("t1", "Music edit", { busy: true }), thread("t3", "Color the ceremony", { lastSentAt: null })]),
  ]);
  return { here, rig };
}

test("the sidebar lists every computer's threads together, working at once, and an offline one's as they were", async ({ page }) => {
  const { here, rig } = await threeMachines(page);
  await page.goto("/");
  const sidebar = page.getByRole("navigation", { name: "Threads" });
  const card = (title: string) => sidebar.getByTestId("sidebar-row-card").filter({ hasText: title });

  // One turn running on each computer: both on the Working shelf, each row saying its computer.
  const working = sidebar.getByTestId("sidebar-working-shelf-toggle");
  await expect(working).toContainText("2");
  await working.click();
  await expect(card("Cut the teaser")).toContainText("Working");
  await expect(card("Cut the teaser").getByTestId("sidebar-row-machine")).toHaveText("Studio-PC");
  await expect(card("Music edit").getByTestId("sidebar-row-machine")).toHaveText("Edit-Rig");
  // The laptop's turn isn't shown as running: nobody can tell.
  await expect(card("Trailer cut").getByTestId("sidebar-row-machine")).toHaveText(/Laptop\s*offline/);
  await expect(card("Trailer cut")).not.toContainText("Working");

  // The Projects menu says which computer each project is on, and scopes the list to one.
  await page.getByRole("button", { name: "Projects" }).click();
  const scopeRow = (name: string) => page.getByTestId("sidebar-scope-project").filter({ hasText: name });
  await expect(scopeRow("Wedding film").getByTestId("sidebar-machine-badge")).toHaveAttribute("title", "On Edit-Rig");
  await expect(scopeRow("Spring promo").getByTestId("sidebar-machine-badge")).toHaveAttribute("title", "On Studio-PC");
  await scopeRow("Wedding film").click();
  await expect(sidebar.getByTestId("sidebar-row-card")).toHaveText([/Color the ceremony/, /Music edit/]);
  await page.getByRole("button", { name: "Projects: Wedding film" }).click();
  await scopeRow("Wedding film").click();
  await expect(card("Cut the teaser")).toBeVisible();

  // A thread on the rig opens there, and the page asks the rig for it.
  await card("Color the ceremony").click();
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Color the ceremony");
  await expect(page.locator("header").getByText("Edit-Rig")).toBeVisible();
  expect(rig).toContainEqual({ type: "select_thread", threadId: "t3" });
  expect(here.filter((message) => message.type === "select_thread")).toEqual([]);

  // An offline computer's thread doesn't open.
  await card("Trailer cut").click();
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Color the ceremony");

  // After a reload the rig picks its own thread for the new connection; the page asks for the one that was open.
  await page.reload();
  await expect(page.getByRole("heading", { level: 1 })).toHaveText("Color the ceremony");
  expect(rig.filter((message) => message.type === "select_thread")).toHaveLength(2);
});

test("a project is added on the computer chosen first, and settings are each computer's own", async ({ page }) => {
  const { here, rig } = await threeMachines(page);
  await page.goto("/");
  await openAddProject(page);

  const computers = page.getByRole("dialog").getByRole("list", { name: "Computers" });
  await expect(computers.getByRole("button")).toHaveText([/Studio-PC/, /Edit-Rig/, /Laptop\s*Offline/]);
  await expect(computers.getByRole("button", { name: /Laptop/ })).toBeDisabled();
  await computers.getByRole("button", { name: /Edit-Rig/ }).click();

  await expect(page.getByRole("dialog").getByRole("button", { name: "On Edit-Rig", exact: true })).toBeVisible();
  await page.getByRole("tab", { name: "Cloud" }).click();
  await page.getByRole("dialog").getByRole("textbox").fill("Launch film");
  await page.getByRole("button", { name: "Add", exact: true }).click();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(rig).toContainEqual({ type: "add_project", source: { kind: "cloud", name: "Launch film" } });
  expect(here.filter((message) => message.type === "add_project")).toEqual([]);

  // Settings open on the computer of the open thread, now the rig's new project.
  await page.getByRole("button", { name: "Settings" }).click();
  const settingsComputers = page.getByRole("navigation", { name: "Settings" }).getByRole("list", { name: "Computers" });
  await expect(page.getByText("Settings of Linker-Edit on")).toContainText("Edit-Rig");
  await expect(settingsComputers.getByRole("button", { name: /Laptop/ })).toBeDisabled();
  await settingsComputers.getByRole("button", { name: /Studio-PC/ }).click();
  await expect(page.getByText("Settings of Linker-Edit on")).toContainText("Studio-PC");
});
