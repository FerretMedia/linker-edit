// The on-disk history format: one folder per timeline, split by domain like Vit
// (github.com/LucasHJin/vit) so separate people's cut, audio and marker changes merge cleanly.

/** What snapshot.py prints. */
export interface RawSnapshot {
  project: string;
  /** Resolve's unique id for the project. */
  project_id?: string | null;
  current_timeline: string | null;
  timelines: RawTimeline[];
}

export interface RawTimeline {
  id: string;
  name: string;
  frame_rate: string | number | null;
  width: string | number | null;
  height: string | number | null;
  start_frame: number | null;
  end_frame: number | null;
  start_timecode: string | null;
  video: RawTrack[];
  audio: RawTrack[];
  markers: ({ frame: number } & Record<string, unknown>)[];
}

interface RawTrack {
  index: number;
  name: string | null;
  items: RawItem[];
}

interface RawItem {
  id: string;
  name: string | null;
  start: number | null;
  end: number | null;
  source_start: number | null;
  source_end: number | null;
  media: { id: string; name: string | null; path: string; fps?: string | number | null } | null;
  enabled: boolean | null;
  properties: Record<string, unknown>;
  fusion_comps?: number;
}

export interface Clip {
  track: number;
  id: string;
  name: string;
  start: number;
  end: number;
  source_start: number | null;
  source_end: number | null;
  /** Media pool item id; null for generators and titles. */
  media: string | null;
  enabled?: false;
  fusion_comps?: number;
  /** Only values that differ from Resolve's defaults. */
  properties?: Record<string, unknown>;
}

export interface Marker {
  frame: number;
  color: string;
  name: string;
  note: string;
  duration: number;
}

export interface TimelineMetadata {
  id: string;
  name: string;
  frame_rate: number;
  width: number;
  height: number;
  start_frame: number;
  end_frame: number;
  start_timecode: string;
  video_tracks: string[];
  audio_tracks: string[];
}

export interface StoredTimeline {
  metadata: TimelineMetadata;
  cuts: Clip[];
  audio: Clip[];
  markers: Marker[];
}

export interface MediaEntry {
  id: string;
  name: string;
  path: string;
  /** The source's own frame rate, which a clip's source frames count in. */
  fps?: number;
}

// Properties worth versioning and their defaults; Vit tracks the same set.
const TRACKED_PROPERTIES: Record<string, unknown> = {
  Pan: 0,
  Tilt: 0,
  ZoomX: 1,
  ZoomY: 1,
  Opacity: 100,
  RotationAngle: 0,
  AnchorPointX: 0,
  AnchorPointY: 0,
  Pitch: 0,
  Yaw: 0,
  FlipX: false,
  FlipY: false,
  CropLeft: 0,
  CropRight: 0,
  CropTop: 0,
  CropBottom: 0,
  CropSoftness: 0,
  CropRetain: false,
  Distortion: 0,
  CompositeMode: 0,
  DynamicZoomEase: 0,
  Speed: 100,
  RetimeProcess: 0,
  MotionEstimation: 0,
  Volume: 0,
};

function isDefault(value: unknown, fallback: unknown): boolean {
  if (typeof fallback === "number") return Number(value) === fallback;
  if (typeof fallback === "boolean") return Boolean(value) === fallback;
  return value === fallback;
}

function trackedProperties(properties: Record<string, unknown>): Record<string, unknown> | undefined {
  const kept = Object.entries(properties).filter(
    ([key, value]) => key in TRACKED_PROPERTIES && value !== null && !isDefault(value, TRACKED_PROPERTIES[key]),
  );
  return kept.length ? Object.fromEntries(kept.sort(([a], [b]) => a.localeCompare(b))) : undefined;
}

const num = (value: unknown, fallback = 0) => (Number.isFinite(Number(value)) ? Number(value) : fallback);

function clipsOf(tracks: RawTrack[]): Clip[] {
  return tracks.flatMap((track) =>
    track.items.map((item) => {
      const clip: Clip = {
        track: track.index,
        id: item.id,
        name: item.name ?? "",
        start: num(item.start),
        end: num(item.end),
        source_start: item.source_start,
        source_end: item.source_end,
        media: item.media?.id ?? null,
      };
      if (item.enabled === false) clip.enabled = false;
      if (item.fusion_comps) clip.fusion_comps = item.fusion_comps;
      const properties = trackedProperties(item.properties);
      if (properties) clip.properties = properties;
      return clip;
    }),
  );
}

export function storedTimeline(raw: RawTimeline): StoredTimeline {
  return {
    metadata: {
      id: raw.id,
      name: raw.name,
      frame_rate: num(raw.frame_rate, 24),
      width: num(raw.width),
      height: num(raw.height),
      start_frame: num(raw.start_frame),
      end_frame: num(raw.end_frame),
      start_timecode: raw.start_timecode ?? "",
      video_tracks: raw.video.map((track) => track.name ?? `V${track.index}`),
      audio_tracks: raw.audio.map((track) => track.name ?? `A${track.index}`),
    },
    cuts: clipsOf(raw.video),
    audio: clipsOf(raw.audio),
    markers: raw.markers
      .map((marker) => ({
        frame: marker.frame,
        color: String(marker.color ?? ""),
        name: String(marker.name ?? ""),
        note: String(marker.note ?? ""),
        duration: num(marker.duration, 1),
      }))
      .sort((a, b) => a.frame - b.frame),
  };
}

export function mediaEntries(snapshot: RawSnapshot): MediaEntry[] {
  const media = new Map<string, MediaEntry>();
  for (const timeline of snapshot.timelines) {
    for (const track of [...timeline.video, ...timeline.audio]) {
      for (const item of track.items) {
        if (!item.media) continue;
        const fps = num(item.media.fps);
        media.set(item.media.id, { id: item.media.id, name: item.media.name ?? "", path: item.media.path, ...(fps ? { fps } : {}) });
      }
    }
  }
  return [...media.values()].sort((a, b) => a.name.localeCompare(b.name) || a.id.localeCompare(b.id));
}

/** Rows go one per line, so a git diff or merge of a cut reads as a clip list. */
export function jsonRows(key: string, rows: unknown[]): string {
  if (rows.length === 0) return `{ "${key}": [] }\n`;
  return `{ "${key}": [\n${rows.map((row) => `  ${JSON.stringify(row)}`).join(",\n")}\n] }\n`;
}

export const TIMELINE_FILES = ["metadata.json", "cuts.json", "audio.json", "markers.json"];

export function timelineFiles(timeline: StoredTimeline): Record<string, string> {
  return {
    "metadata.json": `${JSON.stringify(timeline.metadata, null, 2)}\n`,
    "cuts.json": jsonRows("items", timeline.cuts),
    "audio.json": jsonRows("items", timeline.audio),
    "markers.json": jsonRows("markers", timeline.markers),
  };
}

export function parseTimelineFiles(read: (file: string) => string | null): StoredTimeline | null {
  const metadata = read("metadata.json");
  if (metadata === null) return null;
  const rows = <T>(file: string, key: string): T[] => {
    const text = read(file);
    return text === null ? [] : ((JSON.parse(text) as Record<string, T[]>)[key] ?? []);
  };
  return {
    metadata: JSON.parse(metadata) as TimelineMetadata,
    cuts: rows<Clip>("cuts.json", "items"),
    audio: rows<Clip>("audio.json", "items"),
    markers: rows<Marker>("markers.json", "markers"),
  };
}

/** A folder name that is legal on every OS; "Cut: v2?" becomes "Cut_ v2_". */
export function folderName(name: string): string {
  const safe = name
    .replace(/[<>:"/\\|?*\u0000-\u001f]/g, "_")
    .replace(/[. ]+$/, "")
    .trim();
  return safe || "_";
}

/** Folder per timeline, in timeline order; a name that clashes after cleaning gets " (2)", " (3)"… */
export function timelineFolders(names: string[]): string[] {
  const used = new Set<string>();
  return names.map((name) => {
    const base = folderName(name);
    let folder = base;
    for (let n = 2; used.has(folder.toLowerCase()); n++) folder = `${base} (${n})`;
    used.add(folder.toLowerCase());
    return folder;
  });
}
