<script lang="ts">
  import { Maximize, Minimize, Pause, Play, SkipBack, SkipForward, Volume2, VolumeX } from "@lucide/svelte";
  import type { PlayableTimeline } from "#shared/protocol.ts";
  import { formatTimecode } from "#lib/player.ts";

  let {
    timeline,
    duration,
    time,
    playing,
    muted,
    volume,
    fullscreen,
    onSeek,
    onTogglePlay,
    onCut,
    onToggleMute,
    onVolume,
    onToggleFullscreen,
  }: {
    /** Draws the clips and markers on the scrubber; null shows a plain bar. */
    timeline: PlayableTimeline | null;
    duration: number;
    time: number;
    playing: boolean;
    muted: boolean;
    volume: number;
    fullscreen: boolean;
    onSeek: (time: number) => void;
    onTogglePlay: () => void;
    onCut: (direction: 1 | -1) => void;
    onToggleMute: () => void;
    onVolume: (volume: number) => void;
    onToggleFullscreen: () => void;
  } = $props();

  let bar: HTMLElement | undefined = $state();
  let hover = $state<{ x: number; time: number } | null>(null);
  let dragging = false;

  const fps = $derived(timeline?.fps ?? 24);
  const percent = (seconds: number) => `${duration ? Math.min(100, Math.max(0, (seconds / duration) * 100)) : 0}%`;

  // Resolve's marker colours.
  const MARKER_COLORS: Record<string, string> = {
    Blue: "#3b82f6",
    Cyan: "#22d3ee",
    Green: "#22c55e",
    Yellow: "#facc15",
    Red: "#ef4444",
    Pink: "#f472b6",
    Purple: "#a855f7",
    Fuchsia: "#d946ef",
    Rose: "#fb7185",
    Lavender: "#c4b5fd",
    Sky: "#7dd3fc",
    Mint: "#6ee7b7",
    Lemon: "#fef08a",
    Sand: "#d6c29c",
    Cocoa: "#a16207",
    Cream: "#fef3c7",
  };

  function timeAt(event: PointerEvent): { x: number; time: number } {
    const rect = bar!.getBoundingClientRect();
    const x = Math.min(rect.width, Math.max(0, event.clientX - rect.left));
    return { x, time: rect.width ? (x / rect.width) * duration : 0 };
  }

  function down(event: PointerEvent) {
    dragging = true;
    bar!.setPointerCapture(event.pointerId);
    onSeek(timeAt(event).time);
  }

  function move(event: PointerEvent) {
    hover = timeAt(event);
    if (dragging) onSeek(hover.time);
  }
</script>

<div class="flex flex-col gap-1.5 px-3 pt-2.5 pb-2 text-white">
  <div
    bind:this={bar}
    class="relative h-9 cursor-pointer touch-none overflow-hidden rounded-md bg-white/8 select-none"
    role="slider"
    tabindex="-1"
    aria-label="Position"
    aria-valuemin={0}
    aria-valuemax={duration}
    aria-valuenow={time}
    onpointerdown={down}
    onpointermove={move}
    onpointerup={() => (dragging = false)}
    onpointerleave={() => (hover = null)}
  >
    {#each timeline?.video ?? [] as clip (`${clip.track}:${clip.start}`)}
      <div
        class="absolute overflow-hidden rounded-[3px] border border-black/60 {clip.track > 1
          ? 'top-0 h-2 bg-amber-400/70'
          : 'inset-y-0'} {clip.track === 1 && clip.kind === 'video' ? 'bg-sky-500/35' : ''} {clip.track === 1 &&
        (clip.kind === 'missing' || clip.kind === 'unsupported')
          ? 'bg-white/10'
          : ''}"
        style="left: {percent(clip.start)}; width: {percent(clip.end - clip.start)};{clip.track === 1 &&
        clip.kind === 'image' &&
        clip.media
          ? ` background: center / cover url('/media/${encodeURIComponent(clip.media)}');`
          : ''}"
        title={clip.name}
      ></div>
    {/each}
    <div class="pointer-events-none absolute inset-y-0 left-0 bg-black/45" style="left: {percent(time)}; right: 0"></div>
    {#each timeline?.markers ?? [] as marker, index (index)}
      <span
        class="pointer-events-none absolute -top-px size-2 -translate-x-1/2 rotate-45 rounded-[1px]"
        style="left: {percent(marker.time)}; background: {MARKER_COLORS[marker.color] ?? '#fff'}"
        title={marker.name}
      ></span>
    {/each}
    <div class="pointer-events-none absolute inset-y-0 w-0.5 -translate-x-1/2 bg-white shadow-[0_0_0_1px_rgb(0_0_0/40%)]" style="left: {percent(time)}"></div>
    {#if hover}
      <span
        class="pointer-events-none absolute bottom-0.5 -translate-x-1/2 rounded bg-black/80 px-1 font-mono text-[0.625rem] leading-4"
        style="left: clamp(1.5rem, {hover.x}px, calc(100% - 1.5rem))"
      >
        {formatTimecode(hover.time, fps)}
      </span>
    {/if}
  </div>

  <div class="flex items-center gap-0.5">
    <button type="button" class="flex size-8 items-center justify-center rounded-md hover:bg-white/10" title="Previous cut (↑)" onclick={() => onCut(-1)}>
      <SkipBack class="size-4" />
    </button>
    <button
      type="button"
      class="flex size-8 items-center justify-center rounded-md hover:bg-white/10"
      title={playing ? "Pause (Space)" : "Play (Space)"}
      onclick={onTogglePlay}
    >
      {#if playing}<Pause class="size-4 fill-current" />{:else}<Play class="size-4 fill-current" />{/if}
    </button>
    <button type="button" class="flex size-8 items-center justify-center rounded-md hover:bg-white/10" title="Next cut (↓)" onclick={() => onCut(1)}>
      <SkipForward class="size-4" />
    </button>
    <span class="ms-2 font-mono text-xs tabular-nums">
      {formatTimecode(time, fps)}<span class="text-white/45"> / {formatTimecode(duration, fps)}</span>
    </span>
    <div class="flex-1"></div>
    <button type="button" class="flex size-8 items-center justify-center rounded-md hover:bg-white/10" title={muted ? "Unmute" : "Mute"} onclick={onToggleMute}>
      {#if muted || volume === 0}<VolumeX class="size-4" />{:else}<Volume2 class="size-4" />{/if}
    </button>
    <input
      type="range"
      min="0"
      max="1"
      step="0.05"
      value={muted ? 0 : volume}
      class="w-16 accent-white"
      aria-label="Volume"
      oninput={(event) => onVolume(Number(event.currentTarget.value))}
    />
    <button
      type="button"
      class="ms-1 flex size-8 items-center justify-center rounded-md hover:bg-white/10"
      title={fullscreen ? "Exit full screen (F)" : "Full screen (F)"}
      onclick={onToggleFullscreen}
    >
      {#if fullscreen}<Minimize class="size-4" />{:else}<Maximize class="size-4" />{/if}
    </button>
  </div>
</div>
