import { clipPath, decodeIn, extents, frameRate, readKeyedDict, resolution, versionStamp } from "./blobs.ts";
import { child, childText, descendants, parseXml, textOf, type XmlElement } from "./xml.ts";
import { entryData, readZip, type Zip } from "./zip.ts";

// Reads a Resolve project (.drp) or timeline (.drt) export without Resolve. Both are a ZIP of XML:
// project.xml, MediaPool/**/MpFolder.xml (one per bin), SeqContainer/<uuid>.xml (one per timeline or
// compound clip) and Gallery.xml. Objects are `<Type DbId="uuid">` and refer to each other by DbId.

export interface DrpFile {
  zip: Zip;
  /** Every XML entry's text, by entry name. */
  xml: Map<string, string>;
  project: Project;
  /** The top-level bins, normally just Master. */
  mediaPool: MediaFolder[];
  /** Timelines and compound clips, each with its tracks. */
  timelines: Timeline[];
}

export interface Project {
  /** `<ProjectName>`; a .drt carries the timeline's name here. */
  name: string;
  /** From the version comment: the Resolve that wrote the file, like "21.0.4.0005", and its project format. */
  dbAppVer: string | null;
  dbPrjVer: number | null;
}

export interface MediaFolder {
  id: string;
  name: string;
  folders: MediaFolder[];
  items: MediaItem[];
}

export interface MediaItem {
  id: string;
  /** From the element name: "video", "audio", "timeline", "compound", or the raw name of anything else. */
  kind: string;
  name: string;
  /** The file Resolve links the item to, from its `<Clip>` blob. */
  path: string | null;
  fps: number | null;
  width: number | null;
  height: number | null;
  /** Length in frames at `fps`. */
  frames: number | null;
  /** A timeline or compound clip's first frame. */
  start: number | null;
  /** The `Sm2Sequence` of a timeline or compound clip, which its tracks point at. */
  sequenceId: string | null;
}

export interface Timeline {
  /** The media pool item: a timeline or a compound clip. */
  id: string;
  kind: string;
  name: string;
  /** The SeqContainer entry holding the tracks, and its DbId. */
  entry: string;
  containerId: string;
  sequenceId: string | null;
  fps: number | null;
  width: number | null;
  height: number | null;
  /** First frame, like 86400 for 01:00:00:00 at 24 fps, and length in frames. */
  start: number | null;
  frames: number | null;
  tracks: Track[];
}

export interface Track {
  id: string;
  type: "video" | "audio" | "subtitle" | "other";
  /** 1-based among tracks of the same type, like V1 and A2. */
  index: number;
  name: string;
  clips: TimelineClip[];
}

export interface TimelineClip {
  id: string;
  /** From the element name: "video", "audio", or the raw name of anything else on a track. */
  kind: string;
  name: string;
  /** Record in and out, in timeline frames; out is exclusive. */
  start: number;
  end: number;
  /** Source in and out in timeline frames, from `<In>`; out assumes the clip plays at 100%. */
  sourceIn: number | null;
  sourceOut: number | null;
  /** The media pool item the clip plays. */
  mediaId: string | null;
  /** The plain-text path on the clip. Resolve relinks by the media pool item's path, not this. */
  mediaPath: string | null;
  mediaFps: number | null;
  /**
   * Whether the clip is enabled. Always null for now: no sample file has a disabled clip, so where Resolve
   * stores the flag isn't known yet.
   */
  enabled: boolean | null;
}

export const SEQ_CONTAINER = /(^|\/)SeqContainer(\/[^/]+|\d*)\.xml$/;
export const MP_FOLDER = /^MediaPool\/(.+\/)?MpFolder\.xml$/;

/** Reads a .drp or .drt file's contents. Throws when it isn't one. */
export function readDrp(buffer: Buffer): DrpFile {
  const zip = readZip(buffer);
  const xml = new Map<string, string>();
  for (const entry of zip.entries) {
    if (entry.name.endsWith(".xml")) xml.set(entry.name, entryData(entry).toString("utf8"));
  }
  if (!xml.has("project.xml") && ![...xml.keys()].some((name) => SEQ_CONTAINER.test(name))) {
    throw new Error("Not a Resolve project or timeline: it has no project.xml or timelines.");
  }

  const projectXml = xml.get("project.xml");
  const version = (projectXml && versionStamp(projectXml)) || ([...xml.values()].map(versionStamp).find(Boolean) ?? null);
  const name = projectXml ? childText(projectXml, parseXml(projectXml), "ProjectName") : "";

  const mediaPool = readMediaPool(xml);
  const bySequence = new Map<string, MediaItem>();
  for (const item of mediaItems(mediaPool)) if (item.sequenceId) bySequence.set(item.sequenceId, item);

  const timelines: Timeline[] = [];
  for (const [entry, text] of xml) {
    if (SEQ_CONTAINER.test(entry)) timelines.push(readTimeline(entry, text, bySequence));
  }
  return {
    zip,
    xml,
    project: { name, dbAppVer: version?.dbAppVer ?? null, dbPrjVer: version?.dbPrjVer ?? null },
    mediaPool,
    timelines,
  };
}

/** Every item in the media pool, depth first. */
export function* mediaItems(folders: MediaFolder[]): Generator<MediaItem> {
  for (const folder of folders) {
    yield* folder.items;
    yield* mediaItems(folder.folders);
  }
}

// Each bin is its own MpFolder.xml, naming its parent bin's DbId in <MpFolder>.
function readMediaPool(xml: Map<string, string>): MediaFolder[] {
  const folders: { folder: MediaFolder; parent: string }[] = [];
  for (const [entry, text] of xml) {
    if (!MP_FOLDER.test(entry)) continue;
    const root = parseXml(text);
    const items = (child(root, "MediaVec")?.children ?? []).flatMap((element) => element.children.map((item) => readMediaItem(text, item)));
    folders.push({
      folder: { id: root.id ?? "", name: childText(text, root, "Name"), folders: [], items },
      parent: childText(text, root, "MpFolder"),
    });
  }
  const byId = new Map(folders.map(({ folder }) => [folder.id, folder]));
  const top: MediaFolder[] = [];
  for (const { folder, parent } of folders) {
    const above = byId.get(parent);
    if (above && above !== folder) above.folders.push(folder);
    else top.push(folder);
  }
  return top;
}

function readMediaItem(text: string, element: XmlElement): MediaItem {
  const item: MediaItem = {
    id: element.id ?? "",
    kind: /^Sm2Mp(\w+?)Clip$/.exec(element.name)?.[1]?.toLowerCase() ?? element.name,
    name: childText(text, element, "Name"),
    path: null,
    fps: null,
    width: null,
    height: null,
    frames: null,
    start: null,
    sequenceId: null,
  };
  for (const node of descendants(element)) {
    if (node.name === "Clip" && item.path === null) item.path = attempt(() => clipPath(textOf(text, node)));
    else if (node.name === "BtVideoInfo") {
      const time = attempt(() => readKeyedDict(childText(text, node, "Time")));
      const geometry = attempt(() => readKeyedDict(childText(text, node, "Geometry")));
      const frames = time?.get("NumFrames");
      const rate = time?.get("FrameRate");
      const size = geometry?.get("Resolution");
      if (typeof frames === "number" || typeof frames === "bigint") item.frames = Number(frames);
      if (Buffer.isBuffer(rate)) item.fps = frameRate(rate);
      if (Buffer.isBuffer(size)) Object.assign(item, resolution(size));
    } else if (node.name === "Sm2Sequence" && item.sequenceId === null) {
      item.sequenceId = node.id;
      item.fps = frameRate(hex(text, node, "FrameRate"));
      Object.assign(item, resolution(hex(text, node, "Resolution")));
      const span = extents(hex(text, node, "MediaExtents"));
      if (span && item.fps) {
        item.start = Math.round(span.start * item.fps);
        item.frames = Math.round(span.duration * item.fps);
      }
    }
  }
  return item;
}

function readTimeline(entry: string, text: string, bySequence: Map<string, MediaItem>): Timeline {
  const root = parseXml(text);
  const tracks: Track[] = [];
  const counts = new Map<string, number>();
  for (const track of trackElements(root)) {
    const type = (["video", "audio", "subtitle"] as const)[Number(childText(text, track, "Type"))] ?? "other";
    counts.set(type, (counts.get(type) ?? 0) + 1);
    tracks.push({
      id: track.id ?? "",
      type,
      index: counts.get(type)!,
      name: childText(text, track, "UserDefinedName"),
      clips: clipElements(track).map((clip) => readClip(text, clip)),
    });
  }
  const firstTrack = trackElements(root)[0];
  const sequenceId = firstTrack ? childText(text, firstTrack, "Sequence") || null : null;
  const item = sequenceId ? bySequence.get(sequenceId) : undefined;
  return {
    id: item?.id ?? root.id ?? "",
    kind: item?.kind ?? "timeline",
    name: item?.name ?? "",
    entry,
    containerId: root.id ?? "",
    sequenceId,
    fps: item?.fps ?? null,
    width: item?.width ?? null,
    height: item?.height ?? null,
    start: item?.start ?? null,
    frames: item?.frames ?? null,
    tracks,
  };
}

/** A SeqContainer's `Sm2TiTrack`s, in the order of its `…TrackVec`s. */
export function trackElements(root: XmlElement): XmlElement[] {
  return root.children
    .filter((element) => element.name.endsWith("TrackVec"))
    .flatMap((vec) => vec.children.map((element) => child(element, "Sm2TiTrack")))
    .filter((track) => track !== undefined);
}

/** A track's clips: what sits in each `<Element>` of its `<Items>`. */
export function clipElements(track: XmlElement): XmlElement[] {
  return (child(track, "Items")?.children ?? []).flatMap((element) => element.children);
}

function readClip(text: string, element: XmlElement): TimelineClip {
  const start = Number(childText(text, element, "Start"));
  const end = start + Number(childText(text, element, "Duration"));
  const sourceIn = attempt(() => decodeIn(childText(text, element, "In")));
  return {
    id: element.id ?? "",
    kind: /^Sm2Ti(\w+?)Clip$/.exec(element.name)?.[1]?.toLowerCase() ?? element.name,
    name: childText(text, element, "Name"),
    start,
    end,
    sourceIn,
    sourceOut: sourceIn === null ? null : sourceIn + end - start,
    mediaId: childText(text, element, "MediaRef") || null,
    mediaPath: childText(text, element, "MediaFilePath") || null,
    mediaFps: frameRate(hex(text, element, "MediaFrameRate")),
    enabled: null,
  };
}

function hex(text: string, element: XmlElement, name: string): Buffer {
  return Buffer.from(childText(text, element, name), "hex");
}

function attempt<T>(read: () => T): T | null {
  try {
    return read();
  } catch {
    return null;
  }
}
