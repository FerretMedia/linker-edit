import { expect, test, type Page } from "@playwright/test";
import type { AppState, ComputerStorage, StorageCard } from "../shared/protocol.ts";
import { fakeMachine, project, templateState, thread } from "./fake-machine.ts";

// This computer, measured for real by the e2e server; an edit rig that answers; a laptop that's offline, with the
// report it gave last time.
let template: AppState;

test.beforeAll(async ({}, testInfo) => {
  template = await templateState(String(testInfo.project.use.baseURL));
});

const TB = 1024 ** 4;
const GB = 1024 ** 3;
const rig: ComputerStorage = {
  computerId: "rig",
  disks: [
    { path: "C:\\", label: "C:", totalBytes: 1 * TB, freeBytes: 0.42 * TB },
    { path: "D:\\", label: "D:", totalBytes: 8 * TB, freeBytes: 0.5 * TB },
  ],
  projects: [
    { projectId: "wedding", name: "Wedding film", localBytes: 1.82 * TB, cloudBytes: null, mirror: null },
    { projectId: "promo", name: "Spring promo", localBytes: 212 * GB, cloudBytes: null, mirror: null },
    { projectId: "old", name: "Old trailer", localBytes: null, cloudBytes: null, mirror: null },
  ],
  measuredAt: new Date().toISOString(),
};
const laptop: ComputerStorage = {
  computerId: "laptop",
  disks: [{ path: "/", label: "/", totalBytes: 512 * GB, freeBytes: 96 * GB }],
  projects: [{ projectId: "trailer", name: "Trailer", localBytes: 61.4 * GB, cloudBytes: null, mirror: null }],
  measuredAt: "2026-11-25T18:30:00.000Z",
};

async function threeComputers(page: Page) {
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
  const cards: Record<string, StorageCard> = {
    rig: { state: "fresh", report: rig },
    laptop: { state: "stale", report: laptop, problem: "Couldn't reach Laptop. Is it on, with Linker-Edit running?" },
  };
  await page.route(
    (url) => url.pathname === "/api/machines/storage",
    (route) => route.fulfill({ json: cards[new URL(route.request().url()).searchParams.get("id") ?? ""] }),
  );
  await page.addInitScript(() => localStorage.setItem("linker-edit:machines", JSON.stringify({ laptop: { name: "Laptop", projects: [] } })));
  await fakeMachine(page, "/ws", template, "Studio-PC", [project("promo", "Spring promo", [thread("t1", "Cut the teaser")])]);
  await fakeMachine(page, "/machines/rig/ws", template, "Edit-Rig", [project("wedding", "Wedding film", [thread("t2", "Music edit")])]);
}

test("settings show every computer's disks and projects, and an offline one's last report greyed", async ({ page }) => {
  await threeComputers(page);
  await page.goto("/");
  await page.getByRole("button", { name: "Settings" }).click();
  await page.getByRole("navigation", { name: "Settings" }).getByRole("button", { name: "Computers and storage" }).click();

  const cards = page.getByTestId("storage-card");
  await expect(cards).toHaveCount(3);
  const here = page.getByRole("group", { name: "Studio-PC" });
  const there = page.getByRole("group", { name: "Edit-Rig" });
  const away = page.getByRole("group", { name: "Laptop" });

  // This computer's own disk, measured by its server.
  await expect(here.getByRole("meter").first()).toBeVisible();

  // The rig: a bar per disk, the nearly full one in red, and its projects largest first.
  await expect(there.getByRole("meter")).toHaveCount(2);
  await expect(there.getByRole("list", { name: "Disks on Edit-Rig" })).toContainText("512 GB free of 8.00 TB");
  await expect(there.getByRole("list", { name: "Projects on Edit-Rig" }).getByRole("listitem")).toHaveText([
    /Wedding film\s*1\.82 TB/,
    /Spring promo\s*212 GB/,
    /Old trailer\s*Folder not found/,
  ]);
  await expect(there).toHaveCSS("opacity", "1");

  // The laptop: what it measured last, greyed, with when and why.
  await expect(away).toContainText("Last measured");
  await expect(away).toContainText("Couldn't reach Laptop");
  await expect(away.getByRole("list", { name: "Projects on Laptop" })).toContainText(/Trailer\s*61\.4 GB/);
  await expect(away).toHaveCSS("opacity", "0.6");
});
