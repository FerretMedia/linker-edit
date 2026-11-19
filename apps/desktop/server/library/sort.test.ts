import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { afterAll, describe, expect, it } from "vitest";
import { kindFromFiles, kindFromName, sortFolder } from "./sort.ts";

const root = fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-sort-"));
afterAll(() => fs.rmSync(root, { recursive: true, force: true }));

function write(...parts: string[]) {
  const file = path.join(root, ...parts);
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, "");
}

describe("kindFromName", () => {
  it("knows the usual names of asset folders, however they're written", () => {
    expect(kindFromName("Music")).toBe("music");
    expect(kindFromName("SoundEffects")).toBe("sfx");
    expect(kindFromName("sound-effects")).toBe("sfx");
    expect(kindFromName("SFX")).toBe("sfx");
    expect(kindFromName("VideoFX")).toBe("graphics");
    expect(kindFromName("Video Effects")).toBe("graphics");
    expect(kindFromName("Images")).toBe("graphics");
    expect(kindFromName("Logos")).toBe("graphics");
    expect(kindFromName("LUTs")).toBe("luts");
    expect(kindFromName("B-roll")).toBe("footage");
    expect(kindFromName("Stock Music")).toBe("music");
    expect(kindFromName("Soundtracks")).toBe("music");
  });

  it("says nothing for a name that doesn't tell", () => {
    expect(kindFromName("Audio")).toBeNull();
    expect(kindFromName("Assets")).toBeNull();
    expect(kindFromName("2024")).toBeNull();
  });
});

describe("kindFromFiles", () => {
  const counts = (fields: Partial<Record<"audio" | "video" | "image" | "lut" | "other", number>>) => ({ audio: 0, video: 0, image: 0, lut: 0, other: 0, ...fields });

  it("goes by what most of the files are, but leaves audio to the user", () => {
    expect(kindFromFiles(counts({ image: 8, video: 1 }))).toBe("graphics");
    expect(kindFromFiles(counts({ lut: 3 }))).toBe("luts");
    expect(kindFromFiles(counts({ video: 9, audio: 1 }))).toBe("footage");
    expect(kindFromFiles(counts({ audio: 40 }))).toBeNull();
    expect(kindFromFiles(counts({ image: 5, video: 5 }))).toBeNull();
    expect(kindFromFiles(counts({ other: 5 }))).toBeNull();
  });
});

describe("sortFolder", () => {
  it("sorts a video-editing folder into kinds, splitting a folder whose folders say more than it does", async () => {
    write("Music", "Epidemic", "Upbeat.wav");
    write("Music", "Calm.mp3");
    write("SoundEffects", "Whoosh.wav");
    write("VideoFX", "Light leak.mov");
    write("Images", "logo.png");
    write("Audio", "SFX", "Pop.wav");
    write("Audio", "Voice overs", "Intro.wav");
    write("Overlays and stuff 2", "grain.mov");
    write("Misc", "a.png");
    write("Misc", "b.png");
    write(".cache", "x.bin");
    write("loose.txt");

    const sort = await sortFolder(root);
    expect(sort.root).toBe(path.resolve(root));
    expect(sort.more).toBe(false);
    expect(sort.folders.map(({ name, kind, reason }) => ({ name, kind, reason }))).toEqual([
      { name: "Audio/SFX", kind: "sfx", reason: "name" },
      { name: "Audio/Voice overs", kind: null, reason: null },
      { name: "Images", kind: "graphics", reason: "name" },
      { name: "Misc", kind: "graphics", reason: "files" },
      { name: "Music", kind: "music", reason: "name" },
      { name: "Overlays and stuff 2", kind: "graphics", reason: "name" },
      { name: "SoundEffects", kind: "sfx", reason: "name" },
      { name: "VideoFX", kind: "graphics", reason: "name" },
    ]);
    // Files in folders inside are counted too.
    expect(sort.folders.find((folder) => folder.name === "Music")).toMatchObject({ counts: { audio: 2, video: 0, image: 0, lut: 0, other: 0 }, more: false });
    expect(sort.folders.find((folder) => folder.name === "Audio/SFX")!.path).toBe(path.join(root, "Audio", "SFX"));
  });

  it("says what the folder's own name is, and when it's missing or a file", async () => {
    write("Sounds", "Bank A", "hit.wav");
    expect((await sortFolder(path.join(root, "Sounds"))).rootKind).toBe("sfx");
    await expect(sortFolder(path.join(root, "missing"))).rejects.toThrow(/There is no folder/);
    await expect(sortFolder(path.join(root, "loose.txt"))).rejects.toThrow(/is a file/);
  });
});
