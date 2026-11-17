import { decodeIn, encodeIn, withClipPath } from "./blobs.ts";
import { clipElements, mediaItems, MP_FOLDER, readDrp, SEQ_CONTAINER, trackElements, type DrpFile } from "./read.ts";
import { applySplices, child, childText, descendants, findById, parseXml, setText, textOf, type Splice, type XmlElement } from "./xml.ts";
import { writeZip } from "./zip.ts";

// Cut-level edits to a .drp or .drt. Each edit splices the few XML elements it changes; the rest of every
// entry, blobs included, stays exactly as Resolve wrote it, and entries no edit touches are copied byte for byte.

export type Edit =
  /** Puts the clip at record frame `start`, on track `trackId` of the same timeline and type if given. */
  | { type: "moveClip"; clipId: string; start: number; trackId?: string }
  /** Sets the clip's record in and/or out (out exclusive). Moving the in point moves the source in with it. */
  | { type: "trimClip"; clipId: string; start?: number; end?: number }
  | { type: "removeClip"; clipId: string }
  /** Renames a timeline or compound clip, by its media pool item's id. */
  | { type: "renameTimeline"; timelineId: string; name: string }
  /** Points a media pool item at another file, by rewriting the path in its `<Clip>` blobs. */
  | { type: "relinkMedia"; mediaId: string; path: string };

/**
 * The file with `edits` applied in order, as a new .drp/.drt. Throws, and changes nothing, when an edit names
 * something that isn't there or would leave clips overlapping on a track.
 */
export function patchDrp(file: DrpFile, edits: readonly Edit[]): Buffer {
  const state = new Texts(file.xml);
  const tracks = new Set<string>();
  for (const edit of edits) {
    if (edit.type === "moveClip") tracks.add(moveClip(state, edit.clipId, edit.start, edit.trackId));
    else if (edit.type === "trimClip") tracks.add(trimClip(state, file, edit.clipId, edit.start, edit.end));
    else if (edit.type === "removeClip") removeClip(state, edit.clipId);
    else if (edit.type === "renameTimeline") renameTimeline(state, edit.timelineId, edit.name);
    else if (edit.type === "relinkMedia") relinkMedia(state, edit.mediaId, edit.path);
    else throw new Error(`Unknown edit ${(edit as { type: string }).type}.`);
  }

  const changes = new Map<string, Buffer>();
  for (const [entry, text] of state.texts) {
    if (text !== file.xml.get(entry)) changes.set(entry, Buffer.from(text, "utf8"));
  }
  const buffer = writeZip(file.zip, changes);
  checkTracks(readDrp(buffer), tracks);
  return buffer;
}

/** The XML entries as edited so far, each parsed again only after it changes. */
class Texts {
  texts: Map<string, string>;
  private roots = new Map<string, XmlElement>();

  constructor(xml: Map<string, string>) {
    this.texts = new Map(xml);
  }

  root(entry: string): XmlElement {
    let root = this.roots.get(entry);
    if (!root) {
      root = parseXml(this.texts.get(entry)!);
      this.roots.set(entry, root);
    }
    return root;
  }

  text(entry: string): string {
    return this.texts.get(entry)!;
  }

  splice(entry: string, splices: Splice[]): void {
    this.texts.set(entry, applySplices(this.text(entry), splices));
    this.roots.delete(entry);
  }

  entries(pattern: RegExp): string[] {
    return [...this.texts.keys()].filter((entry) => pattern.test(entry));
  }
}

interface ClipAt {
  entry: string;
  track: XmlElement;
  items: XmlElement;
  /** The `<Element>` in `<Items>` that holds the clip. */
  holder: XmlElement;
  clip: XmlElement;
}

function findClip(state: Texts, clipId: string): ClipAt {
  for (const entry of state.entries(SEQ_CONTAINER)) {
    for (const track of trackElements(state.root(entry))) {
      const items = child(track, "Items");
      for (const holder of items?.children ?? []) {
        const clip = holder.children.find((element) => element.id === clipId);
        if (items && clip) return { entry, track, items, holder, clip };
      }
    }
  }
  throw new Error(`There's no clip ${clipId} on any timeline.`);
}

function frames(value: number, what: string): number {
  if (!Number.isSafeInteger(value) || value < 0) throw new Error(`The ${what} must be a whole number of frames, not ${value}.`);
  return value;
}

function moveClip(state: Texts, clipId: string, start: number, trackId?: string): string {
  frames(start, "start");
  const at = findClip(state, clipId);
  const text = state.text(at.entry);
  let target = at.track;
  if (trackId !== undefined && trackId !== at.track.id) {
    const found = trackElements(state.root(at.entry)).find((track) => track.id === trackId);
    if (!found) throw new Error(`Track ${trackId} isn't on the clip's timeline.`);
    if (childText(text, found, "Type") !== childText(text, at.track, "Type")) {
      throw new Error("A clip can only move to a track of its own type: video to video, audio to audio.");
    }
    target = found;
  }
  const startSplice = setText(text, child(at.clip, "Start")!, String(start));

  // Same track and the clips stay in order: only <Start> changes.
  const others = clipElements(at.track).filter((clip) => clip !== at.clip);
  const index = others.findIndex((clip) => Number(childText(text, clip, "Start")) > start);
  const before = clipElements(at.track).indexOf(at.clip);
  if (target === at.track && (index === -1 ? others.length : index) === before) {
    state.splice(at.entry, [startSplice]);
    return target.id ?? "";
  }

  // Otherwise the clip's <Element> moves, keeping each track's clips in record order like Resolve writes them.
  const [from, to] = lineBlock(text, at.holder);
  const block = applySplices(text.slice(from, to), [{ ...startSplice, start: startSplice.start - from, end: startSplice.end - from }]);
  state.splice(at.entry, [removal(text, at.items, from, to)]);

  const after = state.text(at.entry);
  const track = trackElements(state.root(at.entry)).find((candidate) => candidate.id === target.id)!;
  const items = child(track, "Items");
  if (!items) throw new Error(`Track ${target.id} has no <Items>.`);
  const next = items.children.find((holder) => Number(childText(after, holder.children[0]!, "Start")) > start);
  let insert: Splice;
  if (next) {
    const line = lineStart(after, next.start);
    insert = { start: line, end: line, text: block };
  } else if (items.innerStart === items.end) {
    const indent = after.slice(lineStart(after, items.start), items.start);
    insert = { start: items.start, end: items.end, text: `<${items.name}>${newline(after)}${block}${indent}</${items.name}>` };
  } else {
    const line = lineStart(after, items.innerEnd);
    insert = { start: line, end: line, text: block };
  }
  state.splice(at.entry, [insert]);
  return target.id ?? "";
}

function trimClip(state: Texts, file: DrpFile, clipId: string, start?: number, end?: number): string {
  const at = findClip(state, clipId);
  const text = state.text(at.entry);
  const oldStart = Number(childText(text, at.clip, "Start"));
  const oldEnd = oldStart + Number(childText(text, at.clip, "Duration"));
  const newStart = frames(start ?? oldStart, "start");
  const newEnd = frames(end ?? oldEnd, "end");
  if (newEnd <= newStart) throw new Error(`A clip needs at least one frame: ${newStart} to ${newEnd} has none.`);

  const splices = [setText(text, child(at.clip, "Start")!, String(newStart)), setText(text, child(at.clip, "Duration")!, String(newEnd - newStart))];
  const inElement = child(at.clip, "In");
  let sourceIn = 0;
  if (inElement) {
    try {
      sourceIn = decodeIn(textOf(text, inElement));
    } catch {
      throw new Error(`Clip ${clipId} has a source in point this library can't read, so it can't be trimmed.`);
    }
    sourceIn += newStart - oldStart;
    if (sourceIn < 0) throw new Error(`That trim starts ${-sourceIn} frame(s) before the clip's media does.`);
    if (newStart !== oldStart) splices.push(setText(text, inElement, encodeIn(sourceIn)));
  } else if (newStart !== oldStart) {
    throw new Error(`Clip ${clipId} has no <In>, so its in point can't move.`);
  }

  // Past the end of its media the clip would play nothing; refuse when the media's length is known.
  const timeline = file.timelines.find((candidate) => candidate.entry === at.entry);
  const mediaId = childText(text, at.clip, "MediaRef");
  const media = [...mediaItems(file.mediaPool)].find((item) => item.id === mediaId);
  if (timeline?.fps && media?.fps && media.frames) {
    const length = Math.round((media.frames * timeline.fps) / media.fps);
    if (sourceIn + newEnd - newStart > length) {
      throw new Error(`That trim runs past the end of "${media.name}", which is ${length} frame(s) long on this timeline.`);
    }
  }
  state.splice(at.entry, splices);
  return at.track.id ?? "";
}

function removeClip(state: Texts, clipId: string): void {
  const at = findClip(state, clipId);
  const text = state.text(at.entry);
  const inside = text.slice(at.clip.start, at.clip.end).split(clipId).length - 1;
  const everywhere = [...state.texts.values()].reduce((count, entry) => count + entry.split(clipId).length - 1, 0);
  if (everywhere > inside) throw new Error(`Something else in the file refers to clip ${clipId}, so removing it would break the file.`);
  const [from, to] = lineBlock(text, at.holder);
  state.splice(at.entry, [removal(text, at.items, from, to)]);
}

function renameTimeline(state: Texts, timelineId: string, name: string): void {
  if (!name.trim()) throw new Error("A timeline needs a name.");
  const { entry, item } = findMediaItem(state, timelineId);
  const text = state.text(entry);
  if (!item.children.some((element) => element.name === "Sequence" || element.name === "TimelineSharedHandle")) {
    throw new Error(`Media pool item ${timelineId} isn't a timeline or compound clip.`);
  }
  for (const other of state.entries(MP_FOLDER)) {
    const otherText = state.text(other);
    for (const holder of child(state.root(other), "MediaVec")?.children ?? []) {
      for (const element of holder.children) {
        if (element.id !== timelineId && element.name === item.name && childText(otherText, element, "Name") === name) {
          throw new Error(`There's already a ${item.name === "Sm2MpCompoundClip" ? "compound clip" : "timeline"} called "${name}".`);
        }
      }
    }
  }
  // The media pool item and the Sm2Timeline inside it each carry the name.
  const names = [child(item, "Name"), ...[...descendants(item)].filter((element) => element.name === "Sm2Timeline").map((timeline) => child(timeline, "Name"))];
  state.splice(
    entry,
    names.filter((element) => element !== undefined).map((element) => setText(text, element, name)),
  );
}

function relinkMedia(state: Texts, mediaId: string, path: string): void {
  const { entry, item } = findMediaItem(state, mediaId);
  const text = state.text(entry);
  const blobs = [...descendants(item)]
    .filter((element) => element.name === "BtVideoInfo" || element.name === "BtAudioInfo")
    .map((info) => child(info, "Clip"))
    .filter((clip) => clip !== undefined && textOf(text, clip) !== "");
  if (!blobs.length) throw new Error(`Media pool item ${mediaId} has no file to relink.`);
  state.splice(
    entry,
    blobs.map((clip) => setText(text, clip!, withClipPath(textOf(text, clip), path))),
  );

  // The clips' plain-text paths too, so the file agrees with itself.
  for (const seq of state.entries(SEQ_CONTAINER)) {
    const seqText = state.text(seq);
    const splices: Splice[] = [];
    for (const track of trackElements(state.root(seq))) {
      for (const clip of clipElements(track)) {
        const filePath = child(clip, "MediaFilePath");
        if (filePath && childText(seqText, clip, "MediaRef") === mediaId) splices.push(setText(seqText, filePath, path));
      }
    }
    if (splices.length) state.splice(seq, splices);
  }
}

function findMediaItem(state: Texts, id: string): { entry: string; item: XmlElement } {
  for (const entry of state.entries(MP_FOLDER)) {
    for (const holder of child(state.root(entry), "MediaVec")?.children ?? []) {
      const item = holder.children.find((element) => element.id === id);
      if (item) return { entry, item };
    }
  }
  throw new Error(`There's no media pool item ${id}.`);
}

/** Refuses a result where clips overlap, or have no frames, on a track an edit changed. */
function checkTracks(file: DrpFile, trackIds: Set<string>): void {
  for (const timeline of file.timelines) {
    for (const track of timeline.tracks) {
      if (!trackIds.has(track.id)) continue;
      const clips = [...track.clips].sort((a, b) => a.start - b.start);
      for (let i = 1; i < clips.length; i++) {
        if (clips[i]!.start < clips[i - 1]!.end) {
          const label = `${track.type === "audio" ? "A" : "V"}${track.index}`;
          throw new Error(`That edit makes "${clips[i]!.name}" overlap "${clips[i - 1]!.name}" on ${label} of "${timeline.name}".`);
        }
      }
    }
  }
}

/** Taking text[from, to) out of `items`, or the whole list down to `<Items/>` when it was the last clip. */
function removal(text: string, items: XmlElement, from: number, to: number): Splice {
  if (items.children.length === 1) return { start: items.start, end: items.end, text: `<${items.name}/>` };
  return { start: from, end: to, text: "" };
}

/** An element's whole lines, indentation and line break included, when it has them to itself. */
function lineBlock(text: string, element: XmlElement): [number, number] {
  const from = lineStart(text, element.start);
  if (from === element.start && from > 0 && text[from - 1] !== "\n") return [element.start, element.end];
  const rest = /^[ \t]*\r?\n/.exec(text.slice(element.end, element.end + 64));
  return [from, element.end + (rest?.[0].length ?? 0)];
}

/** Where the line holding `at` starts, if only indentation comes before `at` on it; otherwise `at`. */
function lineStart(text: string, at: number): number {
  let i = at;
  while (i > 0 && (text[i - 1] === " " || text[i - 1] === "\t")) i--;
  return i === 0 || text[i - 1] === "\n" ? i : at;
}

function newline(text: string): string {
  return text.includes("\r\n") ? "\r\n" : "\n";
}
