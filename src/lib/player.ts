import type { PlayableClip, PlayableTimeline } from "#shared/protocol.ts";

export interface PreviewTarget {
  /** Null for Resolve's current timeline. */
  id: string | null;
  /** A save from the history; null for the timeline as it is in Resolve now. */
  commit: string | null;
}

/** "0:06:12" (minutes, seconds, frames), with hours once a timeline runs that long. */
export function formatTimecode(seconds: number, fps: number): string {
  const rate = Math.round(fps) || 24;
  const totalFrames = Math.max(0, Math.round(seconds * rate));
  const frames = totalFrames % rate;
  const whole = Math.floor(totalFrames / rate);
  const pad = (n: number) => String(n).padStart(2, "0");
  const h = Math.floor(whole / 3600);
  const m = Math.floor((whole % 3600) / 60);
  return `${h ? `${h}:${pad(m)}` : m}:${pad(whole % 60)}:${pad(frames)}`;
}

/** The clip showing on each track at a time, bottom track first. */
export function clipsAt(clips: PlayableClip[], time: number): PlayableClip[] {
  const byTrack = new Map<number, PlayableClip>();
  for (const clip of clips) if (clip.start <= time && time < clip.end) byTrack.set(clip.track, clip);
  return [...byTrack.values()].sort((a, b) => a.track - b.track);
}

/** Clips starting soon, loaded ahead so the cut to them doesn't stall. */
export function clipsAhead(clips: PlayableClip[], time: number, seconds = 3): PlayableClip[] {
  return clips.filter((clip) => clip.start > time && clip.start <= time + seconds);
}

export const clipKey = (clip: PlayableClip) => `${clip.track}:${clip.start}`;

/** The previous or next edit point, including the timeline's start and end. */
export function nearestCut(timeline: PlayableTimeline, time: number, direction: 1 | -1): number {
  const cuts = [...new Set([0, timeline.duration, ...timeline.video.flatMap((clip) => [clip.start, clip.end])])].sort((a, b) => a - b);
  // Half a frame of slack, so a playhead parked on a cut moves on to the next one.
  const slack = 0.5 / timeline.fps;
  return direction === 1
    ? (cuts.find((cut) => cut > time + slack) ?? timeline.duration)
    : (cuts.findLast((cut) => cut < time - slack) ?? 0);
}

const number = (value: number | boolean | undefined, fallback: number) => (typeof value === "number" ? value : fallback);

/**
 * Resolve's transform as CSS on a full-frame element. Pan and Tilt are in timeline pixels, Tilt
 * points up and positive rotation turns counter-clockwise.
 */
export function lookStyle(look: PlayableClip["look"], width: number, height: number): string {
  const x = (number(look.Pan, 0) / width) * 100;
  const y = (-number(look.Tilt, 0) / height) * 100;
  const scaleX = number(look.ZoomX, 1) * (look.FlipX ? -1 : 1);
  const scaleY = number(look.ZoomY, 1) * (look.FlipY ? -1 : 1);
  const rotate = -number(look.RotationAngle, 0);
  const opacity = number(look.Opacity, 100) / 100;
  return `transform: translate(${x}%, ${y}%) rotate(${rotate}deg) scale(${scaleX}, ${scaleY}); opacity: ${opacity};`;
}
