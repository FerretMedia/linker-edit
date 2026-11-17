import fs from "node:fs";
import { describe, expect, it } from "vitest";
import { decodeIn, decodeSized, encodeIn, readProto } from "../src/blobs.ts";
import { checkVersion, mediaItems, patchDrp, readDrp, type DrpFile, type Edit } from "../src/index.ts";
import { entryData, readZip } from "../src/zip.ts";

// Fixtures are Resolve exports from davinci-resolve-mcp (MIT, see NOTICE): Resolve 19.1.3 (format 14) and
// 21.0.4 (format 17), a .drp with a sub-bin, one with compound clips and a nested timeline, and a .drt.
const fixture = (name: string) => fs.readFileSync(new URL(`./fixtures/${name}`, import.meta.url));
const BINS = fixture("bins-r19.drp");
const COMPOUND = fixture("compound-r21.drp");
const NESTED = fixture("nested-r19.drt");

function timeline(file: DrpFile, name: string) {
  const found = file.timelines.find((candidate) => candidate.name === name);
  if (!found) throw new Error(`No timeline ${name}`);
  return found;
}

function track(file: DrpFile, timelineName: string, label: string) {
  const type = label[0] === "A" ? "audio" : "video";
  const found = timeline(file, timelineName).tracks.find((candidate) => candidate.type === type && candidate.index === Number(label.slice(1)));
  if (!found) throw new Error(`No track ${label}`);
  return found;
}

function clip(file: DrpFile, timelineName: string, label: string, name: string) {
  const found = track(file, timelineName, label).clips.find((candidate) => candidate.name === name);
  if (!found) throw new Error(`No clip ${name}`);
  return found;
}

const patched = (buffer: Buffer, edits: (file: DrpFile) => Edit[]) => {
  const file = readDrp(buffer);
  const out = patchDrp(file, edits(file));
  return { before: file, out, after: readDrp(out) };
};

describe("reading", () => {
  it("reads a Resolve 19 project: version, media pool bins, media and a timeline", () => {
    const file = readDrp(BINS);
    expect(file.project).toEqual({ name: "_mcp_e89_tpl16", dbAppVer: "19.1.3.0007", dbPrjVer: 14 });

    expect(file.mediaPool.map((folder) => folder.name)).toEqual(["Master"]);
    expect(file.mediaPool[0]!.folders.map((folder) => folder.name)).toEqual(["Archive"]);
    const media = [...mediaItems(file.mediaPool)];
    expect(media.find((item) => item.kind === "video")).toMatchObject({
      name: "r19_template_media.mp4",
      path: "/tmp/r19_template_media.mp4",
      width: 352,
      height: 262,
      frames: 4576,
    });
    expect(media.find((item) => item.kind === "video")!.fps).toBeCloseTo(29.97, 2);

    const edit = timeline(file, "MediaTemplate");
    expect(edit).toMatchObject({ kind: "timeline", fps: 24, width: 1920, height: 1080, start: 86400, frames: 3664 });
    expect(edit.tracks.filter((candidate) => candidate.type === "video")).toHaveLength(1);
    expect(edit.tracks.filter((candidate) => candidate.type === "audio")).toHaveLength(16);
    expect(clip(file, "MediaTemplate", "V1", "r19_template_media.mp4")).toMatchObject({
      kind: "video",
      start: 86400,
      end: 90064,
      sourceIn: 0,
      sourceOut: 3664,
      mediaId: media.find((item) => item.kind === "video")!.id,
      mediaPath: "/tmp/r19_template_media.mp4",
      enabled: null,
    });
  });

  it("reads compound clips and nested timelines as timelines of their own", () => {
    const file = readDrp(COMPOUND);
    expect(file.project).toMatchObject({ name: "ZZ_COMPOUND_RE", dbAppVer: "21.0.4.0005", dbPrjVer: 17 });
    expect(file.timelines.map((candidate) => [candidate.name, candidate.kind]).sort()).toEqual([
      ["CMP_MEDIA", "compound"],
      ["CMP_TITLE", "compound"],
      ["ZZ_CMP_TL", "timeline"],
      ["ZZ_INNER_TL", "timeline"],
    ]);
    expect(timeline(file, "CMP_MEDIA")).toMatchObject({ start: 0, frames: 48 });
    expect(track(file, "ZZ_CMP_TL", "V1").clips.map((candidate) => [candidate.name, candidate.start, candidate.end])).toEqual([
      ["CMP_MEDIA", 86400, 86448],
      ["CMP_TITLE", 86448, 86568],
    ]);
    // A clip of a compound clip or timeline plays that media pool item.
    expect(clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").mediaId).toBe(timeline(file, "ZZ_INNER_TL").id);
  });

  it("reads a .drt the same way, source in points included", () => {
    const file = readDrp(NESTED);
    expect(file.project).toMatchObject({ name: "E57_NESTED", dbPrjVer: 14 });
    expect(file.timelines.map((candidate) => candidate.name).sort()).toEqual(["E57_IN", "E57_NESTED", "E57_OUT"]);
    expect(clip(file, "E57_OUT", "V1", "cut_src.mp4")).toMatchObject({ start: 24, end: 48, sourceIn: 96, sourceOut: 120 });
    expect(clip(file, "E57_NESTED", "V1", "cut_src.mp4")).toMatchObject({ sourceIn: 24, mediaPath: "/Users/samuelgursky/Movies/mcp_probe_tmp/cut_src.mp4" });
  });

  it("encodes <In> the way Resolve writes it", () => {
    // Both values are from the .drt fixture.
    expect(encodeIn(96)).toBe("96|fa7e6abc7493b83f");
    expect(encodeIn(24)).toBe("24|fa7e6abc7493983f");
    expect(encodeIn(0)).toBe("");
    expect(decodeIn("96|fa7e6abc7493b83f")).toBe(96);
  });

  it("refuses what isn't a .drp or .drt", () => {
    expect(() => readDrp(Buffer.from("not a zip at all, just text"))).toThrow(/isn't a ZIP/);
  });
});

describe("writing", () => {
  it("writes an unedited file back byte for byte", () => {
    for (const buffer of [BINS, COMPOUND, NESTED]) expect(patchDrp(readDrp(buffer), []).equals(buffer)).toBe(true);
  });

  it("copies every entry an edit doesn't touch as it was, compressed data included", () => {
    const { out } = patched(COMPOUND, (file) => [{ type: "moveClip", clipId: clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").id, start: 86700 }]);
    const before = readZip(COMPOUND).entries;
    const after = readZip(out).entries;
    expect(after.map((entry) => entry.name)).toEqual(before.map((entry) => entry.name));
    const changed = after.filter((entry, i) => !entry.local.equals(before[i]!.local)).map((entry) => entry.name);
    expect(changed).toEqual([timeline(readDrp(COMPOUND), "ZZ_CMP_TL").entry]);
    // Every entry still passes its CRC check.
    for (const entry of after) expect(() => entryData(entry)).not.toThrow();
  });
});

describe("patching", () => {
  it("moves a clip in time, changing only its <Start>", () => {
    const { before, out, after } = patched(COMPOUND, (file) => [{ type: "moveClip", clipId: clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").id, start: 86700 }]);
    expect(clip(after, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL")).toMatchObject({ start: 86700, end: 86748, sourceIn: 0 });
    const entry = timeline(before, "ZZ_CMP_TL").entry;
    const oldText = before.xml.get(entry)!;
    expect(after.xml.get(entry)).toBe(oldText.replace("<Start>86600</Start>", "<Start>86700</Start>"));
    expect(out.length).toBeGreaterThan(0);
  });

  it("moves a clip to another track, keeping each track in record order", () => {
    const { after } = patched(COMPOUND, (file) => [
      { type: "moveClip", clipId: clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").id, start: 86568, trackId: track(file, "ZZ_CMP_TL", "V1").id },
    ]);
    expect(track(after, "ZZ_CMP_TL", "V1").clips.map((candidate) => [candidate.name, candidate.start])).toEqual([
      ["CMP_MEDIA", 86400],
      ["CMP_TITLE", 86448],
      ["ZZ_INNER_TL", 86568],
    ]);
    expect(track(after, "ZZ_CMP_TL", "V2").clips).toEqual([]);
    expect(after.xml.get(timeline(after, "ZZ_CMP_TL").entry)).toContain("<Items/>");

    // Moving it back, into the now empty track, gives back the XML Resolve wrote.
    const id = clip(after, "ZZ_CMP_TL", "V1", "ZZ_INNER_TL").id;
    const back = readDrp(patchDrp(after, [{ type: "moveClip", clipId: id, start: 86600, trackId: track(after, "ZZ_CMP_TL", "V2").id }]));
    expect(back.xml).toEqual(readDrp(COMPOUND).xml);
  });

  it("reorders clips on their own track when a move passes another", () => {
    const { after } = patched(COMPOUND, (file) => [{ type: "moveClip", clipId: clip(file, "ZZ_CMP_TL", "V1", "CMP_MEDIA").id, start: 86568 }]);
    expect(track(after, "ZZ_CMP_TL", "V1").clips.map((candidate) => [candidate.name, candidate.start])).toEqual([
      ["CMP_TITLE", 86448],
      ["CMP_MEDIA", 86568],
    ]);
  });

  it("refuses moves that overlap another clip or change track type", () => {
    const file = readDrp(COMPOUND);
    const media = clip(file, "ZZ_CMP_TL", "V1", "CMP_MEDIA").id;
    expect(() => patchDrp(file, [{ type: "moveClip", clipId: media, start: 86440 }])).toThrow(/overlap "CMP_MEDIA" on V1/);
    expect(() => patchDrp(file, [{ type: "moveClip", clipId: media, start: 0, trackId: track(file, "ZZ_CMP_TL", "A1").id }])).toThrow(/own type/);
    expect(() => patchDrp(file, [{ type: "moveClip", clipId: "nope", start: 0 }])).toThrow(/no clip nope/);
  });

  it("trims a clip's head and tail, moving its source in with the head", () => {
    const { after } = patched(NESTED, (file) => [{ type: "trimClip", clipId: clip(file, "E57_OUT", "V1", "cut_src.mp4").id, start: 30, end: 40 }]);
    expect(clip(after, "E57_OUT", "V1", "cut_src.mp4")).toMatchObject({ start: 30, end: 40, sourceIn: 102, sourceOut: 112 });
    expect(after.xml.get(timeline(after, "E57_OUT").entry)).toContain(`<In>${encodeIn(102)}</In>`);
  });

  it("refuses trims before the start or past the end of the media", () => {
    const nested = readDrp(NESTED);
    const early = clip(nested, "E57_NESTED", "V1", "cut_src.mp4").id;
    expect(() => patchDrp(nested, [{ type: "trimClip", clipId: early, start: 86370 }])).toThrow(/6 frame\(s\) before/);
    // 4576 frames at 29.97 fps last 3664 frames at 24 fps, all of which the clip already plays.
    const bins = readDrp(BINS);
    const all = clip(bins, "MediaTemplate", "V1", "r19_template_media.mp4").id;
    expect(() => patchDrp(bins, [{ type: "trimClip", clipId: all, end: 90065 }])).toThrow(/past the end/);
    expect(() => patchDrp(bins, [{ type: "trimClip", clipId: all, start: 90064 }])).toThrow(/at least one frame/);
  });

  it("removes a clip and nothing else", () => {
    const { before, after } = patched(NESTED, (file) => [{ type: "removeClip", clipId: clip(file, "E57_NESTED", "V1", "E57_OUT").id }]);
    expect(track(after, "E57_NESTED", "V1").clips.map((candidate) => candidate.name)).toEqual(["cut_src.mp4"]);
    expect(track(after, "E57_NESTED", "A1").clips).toEqual(track(before, "E57_NESTED", "A1").clips);
    expect(after.xml.get(timeline(after, "E57_NESTED").entry)).not.toContain(clip(before, "E57_NESTED", "V1", "E57_OUT").id);
  });

  it("applies several edits to one clip in order", () => {
    const { after } = patched(COMPOUND, (file) => {
      const id = clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").id;
      return [
        { type: "moveClip", clipId: id, start: 86568, trackId: track(file, "ZZ_CMP_TL", "V1").id },
        { type: "trimClip", clipId: id, end: 86590 },
      ];
    });
    expect(clip(after, "ZZ_CMP_TL", "V1", "ZZ_INNER_TL")).toMatchObject({ start: 86568, end: 86590 });
  });

  it("renames a timeline in the media pool and its Sm2Timeline", () => {
    const { before, after } = patched(COMPOUND, (file) => [{ type: "renameTimeline", timelineId: timeline(file, "ZZ_CMP_TL").id, name: "Cut <A> & B" }]);
    expect(after.timelines.map((candidate) => candidate.name)).toContain("Cut <A> & B");
    const pool = [...after.xml].find(([entry]) => entry.startsWith("MediaPool/"))![1];
    expect(pool.split("<Name>Cut &lt;A&gt; &amp; B</Name>")).toHaveLength(3);
    expect(() => patchDrp(before, [{ type: "renameTimeline", timelineId: timeline(before, "ZZ_CMP_TL").id, name: "ZZ_INNER_TL" }])).toThrow(/already a timeline/);
    expect(() => patchDrp(before, [{ type: "renameTimeline", timelineId: [...mediaItems(before.mediaPool)].find((item) => item.kind === "video")!.id, name: "x" }])).toThrow(/isn't a timeline/);
  });

  it("relinks media by rewriting its <Clip> blobs, keeping their other fields", () => {
    const path = "D:\\Proxies\\cut_src.mov";
    const { before, after } = patched(NESTED, (file) => [{ type: "relinkMedia", mediaId: clip(file, "E57_OUT", "V1", "cut_src.mp4").mediaId!, path }]);
    const media = (file: DrpFile) => [...mediaItems(file.mediaPool)].find((item) => item.name === "cut_src.mp4")!;
    expect(media(after).path).toBe(path);
    expect(clip(after, "E57_OUT", "V1", "cut_src.mp4").mediaPath).toBe(path);
    expect(clip(after, "E57_NESTED", "V1", "cut_src.mp4").mediaPath).toBe(path);
    // The other media keeps its path.
    expect([...mediaItems(after.mediaPool)].find((item) => item.name === "white_src.mp4")!.path).toBe("/Users/samuelgursky/Movies/mcp_probe_tmp/white_src.mp4");

    const blobs = (file: DrpFile) => [...file.xml.values()].flatMap((text) => [...text.matchAll(/<Clip>([0-9a-f]+)<\/Clip>/g)].map((match) => match[1]!));
    const old = blobs(before).filter((hex) => readProto(decodeSized(hex).data)[1]!.payload!.toString() === "cut_src.mp4");
    const relinked = blobs(after).filter((hex) => readProto(decodeSized(hex).data)[1]!.payload!.toString() === "cut_src.mov");
    expect(relinked).toHaveLength(old.length);
    expect(old.length).toBe(2); // video and embedded audio
    relinked.forEach((hex, i) => {
      const bytes = Buffer.from(hex, "hex");
      // Version 2, a length that counts the marker and the zstd frame, then marker 0x81 and the frame's magic.
      expect(bytes.readUInt32BE(0)).toBe(2);
      expect(bytes.readUInt32BE(4)).toBe(bytes.length - 8);
      expect(bytes.subarray(8, 13).toString("hex")).toBe("8128b52ffd");
      const fields = readProto(decodeSized(hex).data);
      const oldFields = readProto(decodeSized(old[i]!).data);
      expect(fields.map((field) => field.field)).toEqual(oldFields.map((field) => field.field));
      expect(fields[0]!.payload!.toString()).toBe("D:\\Proxies");
      expect(fields.slice(2).map((field) => field.bytes.toString("hex"))).toEqual(oldFields.slice(2).map((field) => field.bytes.toString("hex")));
    });
  });

  it("relinks back to the original path and gets the same blob contents", () => {
    const file = readDrp(BINS);
    const id = [...mediaItems(file.mediaPool)].find((item) => item.kind === "video")!.id;
    const away = readDrp(patchDrp(file, [{ type: "relinkMedia", mediaId: id, path: "/Volumes/Proxies/a.mov" }]));
    const back = readDrp(patchDrp(away, [{ type: "relinkMedia", mediaId: id, path: "/tmp/r19_template_media.mp4" }]));
    const payloads = (drp: DrpFile) =>
      [...drp.xml.values()].flatMap((text) => [...text.matchAll(/<Clip>([0-9a-f]+)<\/Clip>/g)].map((match) => decodeSized(match[1]!).data.toString("hex")));
    expect(payloads(away)).not.toEqual(payloads(file));
    expect(payloads(back)).toEqual(payloads(file));
    expect(() => patchDrp(file, [{ type: "relinkMedia", mediaId: timeline(file, "MediaTemplate").id, path: "/a.mov" }])).toThrow(/no file to relink/);
  });
});

describe("checkVersion", () => {
  it("passes a file for the same or a newer Resolve", () => {
    expect(checkVersion(readDrp(COMPOUND), 17)).toEqual({ ok: true });
    expect(checkVersion(readDrp(BINS), 17)).toEqual({ ok: true });
    expect(checkVersion(readDrp(NESTED), 14)).toEqual({ ok: true });
  });

  it("explains why an older Resolve can't open a newer file", () => {
    expect(checkVersion(readDrp(COMPOUND), 14)).toEqual({
      ok: false,
      message:
        '"ZZ_COMPOUND_RE" was saved by DaVinci Resolve 21.0.4 in project format 17, but the Resolve opening it reads format 14 ' +
        "(Resolve 19.1). Resolve can't open projects from a newer version: update it to 21.0.4 or later.",
    });
  });

  it("refuses a file whose entries disagree", () => {
    const file = readDrp(COMPOUND);
    file.xml.set("Gallery.xml", file.xml.get("Gallery.xml")!.replace('DbPrjVer="17"', 'DbPrjVer="14"'));
    expect(checkVersion(file, 17)).toMatchObject({ ok: false, message: expect.stringMatching(/mixes project formats 14 and 17/) });
  });
});
