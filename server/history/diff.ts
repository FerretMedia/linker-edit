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

function clipChanges(before: Clip[], after: Clip[], noun: string): string[] {
  const old = new Map(before.map((clip) => [clip.id, clip]));
  const added: Clip[] = [];
  const trimmed: Clip[] = [];
  const moved: Clip[] = [];
  const changed: Clip[] = [];
  for (const clip of after) {
    const previous = old.get(clip.id);
    old.delete(clip.id);
    if (!previous) {
      added.push(clip);
    } else if (
      previous.end - previous.start !== clip.end - clip.start ||
      previous.source_start !== clip.source_start ||
      previous.source_end !== clip.source_end
    ) {
      trimmed.push(clip);
    } else if (previous.start !== clip.start || previous.track !== clip.track) {
      moved.push(clip);
    } else if (
      previous.media !== clip.media ||
      previous.name !== clip.name ||
      previous.enabled !== clip.enabled ||
      previous.fusion_comps !== clip.fusion_comps ||
      !sameJson(previous.properties, clip.properties)
    ) {
      changed.push(clip);
    }
  }
  return [
    phrase(added, noun, "added"),
    phrase([...old.values()], noun, "removed"),
    phrase(trimmed, noun, "trimmed"),
    phrase(moved, noun, "moved"),
    phrase(changed, noun, "changed"),
  ].filter((part) => part !== null);
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
