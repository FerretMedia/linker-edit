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
  // Adding starts with the folder picker, over the form.
  const picker = page.getByRole("dialog", { name: "Choose a library folder" });
  const field = picker.getByRole("combobox", { name: "Library folder" });
  // It fills itself in with the home folder: typed before that arrives, the two would run together.
  await expect(field).not.toHaveValue("");
  await field.fill(music + path.sep);
  // The files in a folder show, greyed out, so you can tell it's the right one.
  await expect(picker.getByText("Tone.wav")).toBeVisible();
  await picker.getByRole("button", { name: "Choose folder" }).click();
  await expect(picker).toHaveCount(0);

  const dialog = page.getByRole("dialog", { name: "Add a folder" });
  await expect(dialog.getByRole("button", { name: `Library folder: ${music}. Change` })).toBeVisible();
  await expect(dialog.getByPlaceholder("music")).toBeVisible();
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

test("an assets folder is sorted into a folder for each kind, checked, then added at once", async ({ page }) => {
  const assets = path.join(os.tmpdir(), "linker-edit-e2e", "video-editing");
  await page.goto("/#library");
  await expect(page.getByRole("heading", { name: "Universal library" })).toBeVisible();

  await page.getByRole("button", { name: "Add folder" }).first().click();
  const picker = page.getByRole("dialog", { name: "Choose a library folder" });
  const field = picker.getByRole("combobox", { name: "Library folder" });
  await expect(field).not.toHaveValue("");
  await field.fill(assets + path.sep);
  await expect(picker.getByRole("option", { name: "SoundEffects" })).toBeVisible();
  await picker.getByRole("button", { name: "Choose folder" }).click();

  // Each folder inside gets a kind from its name; Misc says nothing, so it's left out until a kind is picked.
  const dialog = page.getByRole("dialog", { name: "Add a folder" });
  await expect(dialog.getByRole("combobox", { name: "Kind of Music" })).toHaveValue("music");
  await expect(dialog.getByRole("combobox", { name: "Kind of SoundEffects" })).toHaveValue("sfx");
  await expect(dialog.getByRole("combobox", { name: "Kind of VideoFX" })).toHaveValue("graphics");
  await expect(dialog.getByRole("combobox", { name: "Kind of Images" })).toHaveValue("graphics");
  await expect(dialog.getByRole("combobox", { name: "Kind of Misc" })).toHaveValue("");
  await expect(dialog.getByText("not sure, pick one")).toBeVisible();

  // The user corrects one guess and leaves another out.
  await dialog.getByRole("combobox", { name: "Kind of VideoFX" }).selectOption("footage");
  await dialog.getByRole("combobox", { name: "Kind of Images" }).selectOption("");
  await dialog.getByRole("button", { name: "Add 3 folders" }).click();
  await expect(dialog).toHaveCount(0);

  for (const name of ["Music", "SoundEffects", "VideoFX"]) await expect(page.getByRole("button", { name: `Edit ${name}`, exact: true })).toBeVisible();
  await expect(page.getByRole("button", { name: "Edit Images", exact: true })).toHaveCount(0);
});
