import { expect, test } from "@playwright/test";
import * as os from "node:os";
import * as path from "node:path";

// playwright.config.ts puts a music folder with Tone.wav at <tmp>/linker-edit-e2e/music.
const music = path.join(os.tmpdir(), "linker-edit-e2e", "music");

test("a music folder is added to the library, read, played and starred", async ({ page }) => {
  await page.goto("/");
  await page.getByRole("button", { name: "Library" }).click();
  await expect(page).toHaveURL(/#library/);
  await page.getByRole("button", { name: "Universal" }).click();
  await expect(page.getByRole("heading", { name: "Universal library" })).toBeVisible();

  await page.getByRole("button", { name: "Add folder" }).first().click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("combobox", { name: "Library folder" }).fill(music + path.sep);
  await dialog.getByPlaceholder("Licensed for YouTube only. Use for intros.").fill("Royalty free");
  await dialog.getByRole("button", { name: "Add folder" }).click();
  await expect(dialog).toHaveCount(0);

  // ffmpeg reads its length and loudness.
  const files = page.getByRole("list", { name: "Files" });
  await expect(files.getByText("Tone.wav")).toBeVisible();
  await expect(files.getByText(/^1s · -\d+ LUFS · mono$/)).toBeVisible();
  await expect(page.getByText("Royalty free")).toBeVisible();

  // It plays from this server, only from inside the folder.
  await files.getByRole("button", { name: "Play Tone.wav" }).click();
  const src = await page.locator("audio").getAttribute("src");
  expect(src).toMatch(/^\/library\/[\w-]+\/Tone\.wav$/);
  const response = await page.request.get(src!);
  expect(response.status()).toBe(200);
  expect(response.headers()["content-type"]).toBe("audio/wav");
  expect((await page.request.get(src!.replace("Tone.wav", "..%2F..%2Flinker-edit.config.json"))).status()).toBe(404);

  await files.getByRole("button", { name: "Star Tone.wav" }).click();
  await page.getByRole("button", { name: "Starred" }).click();
  await expect(files.getByRole("button", { name: "Unstar Tone.wav" })).toBeVisible();

  await page.getByRole("textbox", { name: "Search the library" }).fill("nothing like it");
  await expect(files.getByText("Nothing matches.")).toBeVisible();

  await page.getByRole("button", { name: "Back" }).click();
  await expect(page).not.toHaveURL(/#library/);
});
