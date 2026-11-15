import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import type { LibraryFile, LibraryFolder } from "../../shared/protocol.ts";
import { findFfmpeg } from "../transcribe/tools.ts";
import { fileLibraryStore, keepLibrary, memoryLibraryStore, projectFolders, withProjectFolders } from "./folders.ts";
import { createLibrary } from "./library.ts";
import { ffmpegProbe, parseProbe, readCache, scanFolder, type ProbeFile } from "./scan.ts";
import { searchLibrary } from "./search.ts";
import { libraryFile } from "./serve.ts";

const temp = (name: string) => fs.mkdtempSync(path.join(os.tmpdir(), `linker-edit-library-${name}-`));

/** A short 16-bit PCM WAV: a quiet sine, so ffmpeg has something to measure. */
export function wav(seconds: number, channels = 2, rate = 8000): Buffer {
  const samples = Math.round(seconds * rate);
  const data = Buffer.alloc(samples * channels * 2);
  for (let i = 0; i < samples; i += 1) {
    const value = Math.round(Math.sin((2 * Math.PI * 440 * i) / rate) * 8000);
    for (let c = 0; c < channels; c += 1) data.writeInt16LE(value, (i * channels + c) * 2);
  }
  const header = Buffer.alloc(44);
  header.write("RIFF", 0);
  header.writeUInt32LE(36 + data.length, 4);
  header.write("WAVEfmt ", 8);
  header.writeUInt32LE(16, 16);
  header.writeUInt16LE(1, 20);
  header.writeUInt16LE(channels, 22);
  header.writeUInt32LE(rate, 24);
  header.writeUInt32LE(rate * channels * 2, 28);
  header.writeUInt16LE(channels * 2, 32);
  header.writeUInt16LE(16, 34);
  header.write("data", 36);
  header.writeUInt32LE(data.length, 40);
  return Buffer.concat([header, data]);
}

function write(file: string, contents: string | Buffer = "x") {
  fs.mkdirSync(path.dirname(file), { recursive: true });
  fs.writeFileSync(file, contents);
}

/** Makes a link, or says it can't: Windows needs developer mode for links to files. */
function link(target: string, at: string, type: "file" | "junction"): boolean {
  try {
    fs.symlinkSync(target, at, type);
    return true;
  } catch {
    return false;
  }
}

function countingProbe(): ProbeFile & { calls: string[] } {
  const calls: string[] = [];
  const probe = (async (file: string, { loudness }: { loudness: boolean }) => {
    calls.push(path.basename(file));
    return { duration: 12.5, channels: 2, loudness: loudness ? -14 : null };
  }) as ProbeFile & { calls: string[] };
  probe.calls = calls;
  return probe;
}

const folder = (dir: string, kind: LibraryFolder["kind"] = "music", id = "music"): LibraryFolder => ({ id, kind, label: "Music", path: dir, note: "" });

describe("scanning a library folder", () => {
  it("measures new and changed files only, and keeps the rest from its cache", async () => {
    const dir = temp("scan");
    const cacheDir = temp("cache");
    write(path.join(dir, "Upbeat.wav"));
    write(path.join(dir, "Calm", "Piano.mp3"));
    write(path.join(dir, "cover.jpg"));
    write(path.join(dir, ".hidden.wav"));
    const probe = countingProbe();

    const first = await scanFolder(folder(dir), { cacheDir, probe });
    expect(first.files.map((file) => file.rel)).toEqual(["Calm/Piano.mp3", "Upbeat.wav"]);
    expect(first.files[0]).toMatchObject({ duration: 12.5, channels: 2, loudness: -14, measured: true });
    expect(probe.calls.sort()).toEqual(["Piano.mp3", "Upbeat.wav"]);

    probe.calls.length = 0;
    const again = await scanFolder(folder(dir), { cacheDir, probe });
    expect(probe.calls).toEqual([]);
    expect(again.files).toEqual(first.files);

    write(path.join(dir, "Upbeat.wav"), "changed, so longer");
    write(path.join(dir, "New.wav"));
    fs.rmSync(path.join(dir, "Calm"), { recursive: true });
    const third = await scanFolder(folder(dir), { cacheDir, probe });
    expect(probe.calls.sort()).toEqual(["New.wav", "Upbeat.wav"]);
    expect(third.files.map((file) => file.rel)).toEqual(["New.wav", "Upbeat.wav"]);
    expect(readCache(cacheDir, folder(dir))?.files.map((file) => file.rel)).toEqual(["New.wav", "Upbeat.wav"]);

    // A folder that now points elsewhere starts over.
    expect(readCache(cacheDir, { ...folder(dir), path: temp("elsewhere") })).toBeNull();
  });

  it("measures again what ffmpeg wasn't there for", async () => {
    const dir = temp("missing-ffmpeg");
    const cacheDir = temp("cache");
    write(path.join(dir, "Song.wav"));
    let calls = 0;
    const broken: ProbeFile = async () => {
      calls += 1;
      throw new Error("ffmpeg is missing.");
    };
    expect((await scanFolder(folder(dir), { cacheDir, probe: broken })).files[0]).toMatchObject({ duration: null, measured: false });
    await scanFolder(folder(dir), { cacheDir, probe: broken });
    expect(calls).toBe(2);
  });

  it("lists only the files of the folder's kind, and doesn't follow links out of it", async () => {
    const dir = temp("kinds");
    const outside = temp("outside");
    write(path.join(dir, "Logo.png"));
    write(path.join(dir, "Lower third.mov"));
    write(path.join(dir, "Song.wav"));
    write(path.join(dir, "Teal.cube"));
    write(path.join(outside, "Secret.png"));
    const linked = link(outside, path.join(dir, "Linked"), "junction");
    const scan = async (kind: LibraryFolder["kind"]) =>
      (await scanFolder(folder(dir, kind, kind), { cacheDir: temp("cache"), probe: countingProbe() })).files.map((file) => file.rel);
    expect(linked).toBe(true);
    expect(await scan("graphics")).toEqual(["Logo.png", "Lower third.mov"]);
    expect(await scan("luts")).toEqual(["Teal.cube"]);
    expect(await scan("other")).toEqual(["Logo.png", "Lower third.mov", "Song.wav", "Teal.cube"]);
  });

  it("reads duration, channels and loudness from ffmpeg's log", () => {
    const log = `Input #0, wav, from 'song.wav':
  Duration: 00:03:21.50, bitrate: 1411 kb/s
  Stream #0:0: Audio: pcm_s16le ([1][0][0][0] / 0x0001), 44100 Hz, 2 channels, s16, 1411 kb/s
Stream mapping:
  Stream #0:0 -> #0:0 (pcm_s16le (native) -> pcm_s16le (native))
Output #0, null, to 'pipe:':
  Stream #0:0: Audio: pcm_s16le, 44100 Hz, mono, s16, 1411 kb/s
[Parsed_ebur128_0 @ 000002230e3b63c0] Summary:

  Integrated loudness:
    I:         -21.8 LUFS
    Threshold: -31.8 LUFS`;
    expect(parseProbe(log)).toEqual({ duration: 201.5, channels: 2, loudness: -21.8 });
    expect(parseProbe("  Duration: 00:00:10.00, start: 0\n  Stream #0:1(und): Audio: aac (LC), 48000 Hz, 5.1(side), fltp")).toEqual({
      duration: 10,
      channels: 6,
      loudness: null,
    });
    expect(parseProbe("  Duration: N/A, bitrate: N/A\n  Stream #0:0: Video: png, rgba, 640x360")).toEqual({ duration: null, channels: null, loudness: null });
  });

  it("measures a real file with the ffmpeg Linker-Edit ships", async () => {
    const dir = temp("ffmpeg");
    write(path.join(dir, "Tone.wav"), wav(2, 2));
    const probe = ffmpegProbe(findFfmpeg);
    const measured = await probe(path.join(dir, "Tone.wav"), { loudness: true });
    expect(measured.duration).toBe(2);
    expect(measured.channels).toBe(2);
    expect(measured.loudness).toBeLessThan(-5);
    expect(measured.loudness).toBeGreaterThan(-40);
    expect(await probe(path.join(dir, "Tone.wav"), { loudness: false })).toEqual({ duration: 2, channels: 2, loudness: null });
  });
});

function file(fields: Partial<LibraryFile> & { rel: string }): LibraryFile {
  return {
    folderId: "music",
    scope: "universal",
    kind: "music",
    name: fields.rel.split("/").at(-1)!,
    path: `/library/${fields.rel}`,
    size: 1,
    modified: "2026-11-15T10:00:00.000Z",
    duration: 120,
    channels: 2,
    loudness: -14,
    tags: [],
    star: false,
    note: "",
    usedIn: [],
    ...fields,
  };
}

describe("searching the library", () => {
  const files = [
    file({ rel: "Upbeat_Summer-Vibes.mp3", duration: 150, tags: ["happy"] }),
    file({ rel: "Calm/Piano.wav", duration: 200, note: "upbeat at the end" }),
    file({ rel: "Whoosh 1.wav", kind: "sfx", folderId: "sfx", duration: 1.2, tags: ["Transition"] }),
    file({ rel: "Whoosh 2.wav", kind: "sfx", folderId: "sfx", duration: 0.8, tags: ["transition"], star: true }),
    file({ rel: "Logo.png", kind: "graphics", folderId: "logos", duration: null }),
    file({ rel: "Theme.wav", scope: "project", folderId: "own", duration: 90 }),
  ];
  const names = (query: Parameters<typeof searchLibrary>[1], order: string[] = []) => searchLibrary(files, query, order).map((entry) => entry.name);

  it("finds words in names first, then in tags and notes, ignoring _ and -", () => {
    expect(names({ query: "upbeat" })).toEqual(["Upbeat_Summer-Vibes.mp3", "Piano.wav"]);
    expect(names({ query: "summer vibes" })).toEqual(["Upbeat_Summer-Vibes.mp3"]);
    expect(names({ query: "happy" })).toEqual(["Upbeat_Summer-Vibes.mp3"]);
    expect(names({ query: "calm" })).toEqual(["Piano.wav"]);
    expect(names({ query: "upbeat nothing" })).toEqual([]);
  });

  it("filters by kind, tags (all of them, any case), stars and length", () => {
    expect(names({ kind: "sfx" })).toEqual(["Whoosh 2.wav", "Whoosh 1.wav"]);
    expect(names({ tags: ["transition"] })).toEqual(["Whoosh 2.wav", "Whoosh 1.wav"]);
    expect(names({ tags: ["transition", "happy"] })).toEqual([]);
    expect(names({ starred: true })).toEqual(["Whoosh 2.wav"]);
    expect(names({ minSeconds: 1, maxSeconds: 160 })).toEqual(["Theme.wav", "Upbeat_Summer-Vibes.mp3", "Whoosh 1.wav"]);
    // A still has no length, so a length range leaves it out.
    expect(names({ maxSeconds: 10_000 })).not.toContain("Logo.png");
  });

  it("puts the most specific library first", () => {
    expect(names({ kind: "music" }, ["universal", "project"])[0]).toBe("Theme.wav");
    expect(names({ kind: "music" }, ["project", "universal"]).at(-1)).toBe("Theme.wav");
  });
});

describe("the files the library page may play", () => {
  const dir = temp("serve");
  const outside = temp("serve-outside");
  write(path.join(dir, "Song.mp3"));
  write(path.join(dir, "Sub", "Logo.png"));
  write(path.join(dir, "page.html"), "<script>alert(1)</script>");
  write(path.join(dir, "logo.svg"), "<svg onload='alert(1)'/>");
  write(path.join(dir, ".hidden.mp3"));
  write(path.join(outside, "Secret.mp3"));
  const linkedFolder = link(outside, path.join(dir, "Out"), "junction");
  const linkedFile = link(path.join(outside, "Secret.mp3"), path.join(dir, "Secret.mp3"), "file");
  const folders = [folder(dir, "music", "lib")];
  const serves = (rel: string, id = "lib") => libraryFile(folders, id, rel)?.path ?? null;

  it("serves playable files inside the folder", () => {
    expect(serves("Song.mp3")).toBe(fs.realpathSync.native(path.join(dir, "Song.mp3")));
    expect(libraryFile(folders, "lib", "Sub/Logo.png")?.type).toBe("image/png");
  });

  it.each([
    ["../" + path.basename(outside) + "/Secret.mp3"],
    ["Sub/../../" + path.basename(outside) + "/Secret.mp3"],
    ["Sub\\..\\..\\" + path.basename(outside) + "\\Secret.mp3"],
    ["./Song.mp3"],
    [path.join(outside, "Secret.mp3")],
    ["/etc/hosts.mp3"],
    ["C:/Windows/Media/tada.wav"],
    ["C:Song.mp3"],
    ["D:\\Music\\Song.mp3"],
    ["\\\\server\\share\\Song.mp3"],
    [".hidden.mp3"],
    ["Missing.mp3"],
    ["Sub"],
    ["page.html"],
    ["logo.svg"],
    ["Song.mp3:stream"],
    [""],
  ])("refuses %s", (rel) => {
    expect(serves(rel)).toBeNull();
  });

  it("refuses a folder it doesn't know", () => {
    expect(serves("Song.mp3", "other")).toBeNull();
  });

  it("refuses files that links lead out of the folder to", () => {
    expect(linkedFolder).toBe(true);
    expect(serves("Out/Secret.mp3")).toBeNull();
    if (linkedFile) expect(serves("Secret.mp3")).toBeNull();
  });
});

describe("library stores", () => {
  it("keeps folders and what's noted on files in library.json, and puts a broken one aside", () => {
    const dir = temp("store");
    const file = path.join(dir, "library.json");
    const store = fileLibraryStore(file);
    store.saveFolders([folder("C:\\Music")]);
    store.updateData("C:\\Music\\Song.wav", (data) => ({ ...data, star: true, tags: ["happy"] }));
    const again = fileLibraryStore(file);
    expect(again.folders()).toEqual([folder(path.resolve("C:\\Music"))]);
    expect(again.data("C:\\Music\\Song.wav")).toMatchObject({ star: true, tags: ["happy"] });
    // Nothing noted any more: the file's entry goes.
    again.updateData("C:\\Music\\Song.wav", (data) => ({ ...data, star: false, tags: [] }));
    expect(JSON.parse(fs.readFileSync(file, "utf8")).files).toEqual({});

    fs.writeFileSync(file, "{ not json");
    const errors: unknown[] = [];
    expect(fileLibraryStore(file, (error) => errors.push(error)).folders()).toEqual([]);
    expect(errors).toHaveLength(1);
    expect(fs.readdirSync(dir).some((name) => name.startsWith("library.json.bad-"))).toBe(true);
  });

  it("keeps a project's folders in its local link, also when the link changes", () => {
    const local = withProjectFolders({ database: { type: "Disk", name: "Local" } }, [folder("D:\\Own")]);
    expect(projectFolders(local).map((entry) => entry.id)).toEqual(["music"]);
    expect(projectFolders(keepLibrary(local, { database: { type: "Disk", name: "Other" } }))).toHaveLength(1);
    expect(withProjectFolders(local, [])).toEqual({ database: { type: "Disk", name: "Local" } });
    expect(projectFolders({ library: [{ id: "x", kind: "nonsense", path: "D:\\" }] } as never)).toEqual([]);
  });
});

describe("the library service", () => {
  it("finds a file by its path in the most specific library, with what's noted on it", async () => {
    const shared = temp("service-shared");
    const own = temp("service-own");
    write(path.join(shared, "Song.wav"));
    write(path.join(own, "Theme.wav"));
    const store = memoryLibraryStore([folder(shared, "music", "shared")]);
    const library = createLibrary({ store, cacheDir: temp("cache"), probe: countingProbe() });
    const ownFolder = { ...folder(own, "music", "own"), label: "Ours" };
    await library.scanAll([...library.folders(), ownFolder]);
    const chain = [
      { scope: { kind: "universal" as const }, label: "Universal", folders: library.folders() },
      { scope: { kind: "project" as const, projectId: "p1" }, label: "Doc", folders: [ownFolder] },
    ];
    library.updateFile(path.join(shared, "Song.wav"), { star: true });
    expect(library.find(chain, path.join(shared, "Song.wav"))).toMatchObject({ scope: "universal", star: true, rel: "Song.wav" });
    expect(library.find(chain, path.join(own, "Theme.wav"))).toMatchObject({ scope: "project", folderId: "own" });
    expect(library.find(chain, path.join(own, "Nope.wav"))).toBeNull();
    expect(library.find(chain.slice(0, 1), path.join(own, "Theme.wav"))).toBeNull();
    expect(library.search(chain, { kind: "music" }).map((entry) => entry.name)).toEqual(["Theme.wav", "Song.wav"]);
  });
});
