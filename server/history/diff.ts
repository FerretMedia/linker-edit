import type { ClipChange, StripTile, TimelineStrip } from "../../shared/protocol.ts";
import type { Clip, Marker, StoredTimeline } from "./model.ts";

export function formatDuration(frames: number, fps: number): string {
  const seconds = Math.max(0, Math.round(frames / (fps || 24)));
  const h = Math.floor(seconds / 3600);
  const m = Math.floor((seconds % 3600) / 60);
  const s = String(seconds % 60).padStart(2, "0");
  return h ? `${h}:${String(m).padStart(2, "0")}:${s}` : `${m}:${s}`;
}

const length = (timeline: StoredTimeline) =>
  formatDuration(timeline.metadata.end_frame - timeline.metadata.start_frame, timeline.metadata.frame_rate);

function count(n: number, singular: string, plural = `${singular}s`) {
  return `${n} ${n === 1 ? singular : plural}`;
}

/** "3 clips removed (A001, A002, B003)": names only while the list stays short. */
function phrase(clips: Clip[], noun: string, verb: string): string | null {
  if (clips.length === 0) return null;
  const names = clips.length <= 3 ? ` (${clips.map((clip) => clip.name || "untitled").join(", ")})` : "";
  return `${count(clips.length, noun)} ${verb}${names}`;
}

const sameJson = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

interface ComparedClip {
  clip: Clip;
  previous?: Clip;
  change: ClipChange | null;
}

/** Every clip after the change, then the removed ones, each with what happened to it. Trimmed beats moved beats changed. */
function compareClips(before: Clip[], after: Clip[]): ComparedClip[] {
  const old = new Map(before.map((clip) => [clip.id, clip]));
  const unmatched = after.filter((clip) => !old.has(clip.id));
  const matched = new Map(after.flatMap((clip) => (old.has(clip.id) ? [[clip.id, old.get(clip.id)!] as const] : [])));
  for (const clip of matched.values()) old.delete(clip.id);
  // Resolve gives a clip a new id whenever it is placed again, which is how scripts and agents move or
  // re-trim one. The same media from the same source in or out point is still that clip.
  const pairs = new Map<Clip, Clip>();
  for (const clip of unmatched) {
    const twin = [...old.values()].find(
      (candidate) =>
        clip.media !== null &&
        candidate.media === clip.media &&
        (candidate.source_start === clip.source_start || candidate.source_end === clip.source_end),
    );
    if (!twin) continue;
    old.delete(twin.id);
    pairs.set(clip, twin);
  }
  const compared = after.map((clip): ComparedClip => {
    const previous = matched.get(clip.id) ?? pairs.get(clip);
    if (!previous) return { clip, change: "added" };
    let change: ClipChange | null = null;
    if (
      previous.end - previous.start !== clip.end - clip.start ||
      previous.source_start !== clip.source_start ||
      previous.source_end !== clip.source_end
    ) {
      change = "trimmed";
    } else if (previous.start !== clip.start || previous.track !== clip.track) {
      change = "moved";
    } else if (
      previous.media !== clip.media ||
      previous.name !== clip.name ||
      previous.enabled !== clip.enabled ||
      previous.fusion_comps !== clip.fusion_comps ||
      !sameJson(previous.properties, clip.properties)
    ) {
      change = "changed";
    }
    return { clip, previous, change };
  });
  const removed = before.filter((clip) => old.has(clip.id));
  return [...compared, ...removed.map((clip): ComparedClip => ({ clip, change: "removed" }))];
}

const VERBS: [ClipChange, string][] = [
  ["added", "added"],
  ["removed", "removed"],
  ["trimmed", "trimmed"],
  ["moved", "moved"],
  ["changed", "changed"],
];

function clipChanges(before: Clip[], after: Clip[], noun: string): string[] {
  const compared = compareClips(before, after);
  return VERBS.map(([change, verb]) =>
    phrase(
      compared.filter((entry) => entry.change === change).map((entry) => entry.clip),
      noun,
      verb,
    ),
  ).filter((part) => part !== null);
}

function markerChanges(before: Marker[], after: Marker[]): string[] {
  const old = new Map(before.map((marker) => [marker.frame, marker]));
  let added = 0;
  let changed = 0;
  for (const marker of after) {
    const previous = old.get(marker.frame);
    old.delete(marker.frame);
    if (!previous) added++;
    else if (!sameJson(previous, marker)) changed++;
  }
  return [
    added && `${count(added, "marker")} added`,
    old.size && `${count(old.size, "marker")} removed`,
    changed && `${count(changed, "marker")} changed`,
  ].filter((part) => typeof part === "string");
}

function timelineChanges(before: StoredTimeline, after: StoredTimeline): string[] {
  const a = before.metadata;
  const b = after.metadata;
  const parts: string[] = [];
  if (a.name !== b.name) parts.push(`renamed from "${a.name}"`);
  parts.push(...clipChanges(before.cuts, after.cuts, "clip"));
  parts.push(...clipChanges(before.audio, after.audio, "audio clip"));
  parts.push(...markerChanges(before.markers, after.markers));
  if (!sameJson(a.video_tracks, b.video_tracks) || !sameJson(a.audio_tracks, b.audio_tracks)) {
    parts.push("tracks changed");
  }
  if (a.frame_rate !== b.frame_rate || a.width !== b.width || a.height !== b.height) {
    parts.push("timeline settings changed");
  }
  if (parts.length && length(before) !== length(after)) parts.push(`length ${length(before)} → ${length(after)}`);
  return parts;
}

/** One line per timeline that changed, matched by Resolve's timeline id so renames read as renames. */
export function describeChanges(before: StoredTimeline[], after: StoredTimeline[]): string[] {
  const old = new Map(before.map((timeline) => [timeline.metadata.id, timeline]));
  const lines: string[] = [];
  for (const timeline of after) {
    const previous = old.get(timeline.metadata.id);
    old.delete(timeline.metadata.id);
    const name = `"${timeline.metadata.name}"`;
    if (!previous) {
      lines.push(`${name}: new timeline, ${count(timeline.cuts.length, "clip")}, ${length(timeline)}`);
      continue;
    }
    const parts = timelineChanges(previous, timeline);
    if (parts.length) lines.push(`${name}: ${parts.join(", ")}`);
  }
  for (const timeline of old.values()) lines.push(`"${timeline.metadata.name}": timeline removed`);
  return lines;
}

/** Tiles shown before a strip folds the rest into "+N". */
const STRIP_LIMIT = 24;

const seconds = (frames: number, fps: number) => Math.round((frames / (fps || 24)) * 10) / 10;

/**
 * Video clips in timeline order for the history panel. A new timeline shows its whole cut; a changed
 * one shows each changed clip with a neighbour either side and folds the untouched runs between.
 */
export function timelineStrips(
  before: StoredTimeline[],
  after: StoredTimeline[],
  isImage: (media: string) => boolean,
): TimelineStrip[] {
  const old = new Map(before.map((timeline) => [timeline.metadata.id, timeline]));
  const strips: TimelineStrip[] = [];
  for (const timeline of after) {
    const previous = old.get(timeline.metadata.id);
    const fps = timeline.metadata.frame_rate;
    const compared = previous
      ? compareClips(previous.cuts, timeline.cuts)
      : timeline.cuts.map((clip): ComparedClip => ({ clip, change: null }));
    if (previous && !compared.some((entry) => entry.change)) continue;
    if (compared.length === 0) continue;

    const at = (entry: ComparedClip) => entry.clip.start;
    compared.sort((a, b) => at(a) - at(b) || a.clip.track - b.clip.track);
    const tile = ({ clip, previous: was, change }: ComparedClip): StripTile => {
      const delta =
        change === "moved" && was
          ? seconds(clip.start - was.start, fps)
          : change === "trimmed" && was
            ? seconds(clip.end - clip.start - (was.end - was.start), fps)
            : undefined;
      return {
        kind: "clip",
        name: clip.name,
        media: clip.media,
        image: clip.media !== null && isImage(clip.media),
        change,
        ...(delta ? { delta } : {}),
      };
    };

    const keep = compared.map(
      (_, index) => !previous || [index - 1, index, index + 1].some((near) => compared[near]?.change),
    );
    const tiles: StripTile[] = [];
    let shown = 0;
    for (let index = 0; index < compared.length; ) {
      let run = 0;
      while (index + run < compared.length && !keep[index + run]) run++;
      // A lone untouched clip shows as itself: a "+1" tile would take the same room.
      if (run === 1) keep[index] = true;
      if (run > 1) {
        tiles.push({ kind: "more", count: run });
        index += run;
        continue;
      }
      if (shown === STRIP_LIMIT) {
        tiles.push({ kind: "more", count: compared.length - index });
        break;
      }
      tiles.push(tile(compared[index]!));
      shown++;
      index++;
    }
    strips.push({ id: timeline.metadata.id, name: timeline.metadata.name, state: previous ? "changed" : "new", tiles });
  }
  return strips;
}
