import * as path from "node:path";
import type { MediaKind, PlayableClip, PlayableTimeline } from "../../shared/protocol.ts";
import type { Clip, MediaEntry, StoredTimeline } from "./model.ts";

// Files a browser can show or play, by extension. A .mov may still hold a codec browsers can't
// decode, such as ProRes; the player shows those as unplayable when they fail to load.
const MEDIA_TYPES: Record<string, { kind: MediaKind; type: string }> = {
  ".jpg": { kind: "image", type: "image/jpeg" },
  ".jpeg": { kind: "image", type: "image/jpeg" },
  ".png": { kind: "image", type: "image/png" },
  ".webp": { kind: "image", type: "image/webp" },
  ".gif": { kind: "image", type: "image/gif" },
  ".bmp": { kind: "image", type: "image/bmp" },
  ".avif": { kind: "image", type: "image/avif" },
  ".mp4": { kind: "video", type: "video/mp4" },
  ".m4v": { kind: "video", type: "video/mp4" },
  ".mov": { kind: "video", type: "video/quicktime" },
  ".webm": { kind: "video", type: "video/webm" },
  ".mp3": { kind: "audio", type: "audio/mpeg" },
  ".m4a": { kind: "audio", type: "audio/mp4" },
  ".aac": { kind: "audio", type: "audio/aac" },
  ".wav": { kind: "audio", type: "audio/wav" },
  ".ogg": { kind: "audio", type: "audio/ogg" },
  ".flac": { kind: "audio", type: "audio/flac" },
};

export const mediaType = (file: string | undefined) => (file ? (MEDIA_TYPES[path.extname(file).toLowerCase()] ?? null) : null);

// The properties the quick preview can approximate with CSS.
const LOOK = ["ZoomX", "ZoomY", "Pan", "Tilt", "RotationAngle", "Opacity", "FlipX", "FlipY"];

export function playableTimeline(
  timeline: StoredTimeline,
  media: Map<string, MediaEntry>,
  commit: string | null,
): PlayableTimeline {
  const { metadata } = timeline;
  const fps = metadata.frame_rate || 24;
  const seconds = (frame: number) => (frame - metadata.start_frame) / fps;

  const clip = (source: Clip): PlayableClip => {
    const entry = source.media ? media.get(source.media) : undefined;
    const file = entry?.path;
    const properties = source.properties ?? {};
    const look = Object.fromEntries(
      LOOK.filter((key) => key in properties).map((key) => [key, properties[key] as number | boolean]),
    );
    return {
      name: source.name,
      track: source.track,
      start: seconds(source.start),
      end: seconds(source.end),
      // Source frames count at the media's own rate: 25 fps footage on a 24 fps timeline is common.
      offset: Math.max(0, source.source_start ?? 0) / (entry?.fps || fps),
      rate: Number(properties.Speed ?? 100) / 100 || 1,
      media: file ? source.media : null,
      kind: source.media === null ? "missing" : (mediaType(file)?.kind ?? (file ? "unsupported" : "missing")),
      look,
    };
  };
  const playable = (clips: Clip[]) =>
    clips.filter((source) => source.enabled !== false).map(clip).sort((a, b) => a.start - b.start || a.track - b.track);

  return {
    id: metadata.id,
    name: metadata.name,
    commit,
    fps,
    width: metadata.width || 1920,
    height: metadata.height || 1080,
    duration: Math.max(0, (metadata.end_frame - metadata.start_frame) / fps),
    video: playable(timeline.cuts),
    audio: playable(timeline.audio),
    markers: timeline.markers.map((marker) => ({ time: marker.frame / fps, color: marker.color, name: marker.name })),
  };
}
