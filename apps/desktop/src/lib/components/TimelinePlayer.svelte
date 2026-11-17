<script lang="ts">
  import { ImageOff } from "@lucide/svelte";
  import type { PlayableClip, PlayableTimeline } from "#shared/protocol.ts";
  import { clipKey, clipsAhead, clipsAt, lookStyle } from "#lib/player.ts";

  let {
    timeline,
    time = $bindable(0),
    playing = $bindable(false),
    muted,
    volume,
  }: {
    timeline: PlayableTimeline;
    time: number;
    playing: boolean;
    muted: boolean;
    volume: number;
  } = $props();

  let failed = $state(new Set<string>());

  const playable = (clip: PlayableClip) => clip.media !== null && !failed.has(clip.media) && clip.kind !== "missing" && clip.kind !== "unsupported";
  const src = (clip: PlayableClip) => `/media/${encodeURIComponent(clip.media ?? "")}`;

  // What is on screen now, plus what comes up next so it is loaded before the cut.
  const pictures = $derived.by(() => {
    const now = clipsAt(timeline.video, time);
    const active = new Set(now);
    return [...now, ...clipsAhead(timeline.video, time)].map((clip) => ({ clip, active: active.has(clip) }));
  });
  const sounds = $derived.by(() => {
    const audible = timeline.audio.filter((clip) => playable(clip) && (clip.kind === "audio" || clip.kind === "video"));
    const now = audible.filter((clip) => clip.start <= time && time < clip.end);
    const active = new Set(now);
    return [...now, ...clipsAhead(audible, time)].map((clip) => ({ clip, active: active.has(clip) }));
  });

  $effect(() => {
    // The play clock. It only runs while playing; the media elements follow it.
    if (!playing) return;
    let last = performance.now();
    let frame = requestAnimationFrame(function tick(now) {
      time = Math.min(timeline.duration, time + (now - last) / 1000);
      last = now;
      if (time >= timeline.duration) playing = false;
      else frame = requestAnimationFrame(tick);
    });
    return () => cancelAnimationFrame(frame);
  });

  interface Sync {
    clip: PlayableClip;
    active: boolean;
    time: number;
    playing: boolean;
    muted?: boolean;
    volume?: number;
  }

  /** Keeps a video or audio element at the clock's position for its clip. */
  function sync(element: HTMLMediaElement, params: Sync) {
    function apply({ clip, active, time, playing, muted, volume }: Sync) {
      if (muted !== undefined) element.muted = muted;
      if (volume !== undefined) element.volume = volume;
      if (element.readyState === 0) return;
      if (!active) {
        if (!element.paused) element.pause();
        if (Math.abs(element.currentTime - clip.offset) > 0.1) element.currentTime = clip.offset;
        return;
      }
      element.playbackRate = clip.rate;
      const wanted = clip.offset + (time - clip.start) * clip.rate;
      // Playing tolerates a little drift instead of seeking every frame; a paused scrub lands on the frame.
      if (Math.abs(element.currentTime - wanted) > (playing ? 0.3 : 0.02)) element.currentTime = wanted;
      if (playing && element.paused) element.play().catch(() => {});
      if (!playing && !element.paused) element.pause();
    }
    const ready = () => apply(params);
    element.addEventListener("loadedmetadata", ready);
    apply(params);
    return {
      update(next: Sync) {
        params = next;
        apply(next);
      },
      destroy: () => element.removeEventListener("loadedmetadata", ready),
    };
  }

  function fail(clip: PlayableClip) {
    if (clip.media) failed = new Set(failed).add(clip.media);
  }

  const extension = (name: string) => /\.([a-z0-9]+)$/i.exec(name)?.[1]?.toUpperCase() ?? "This file";
</script>

<div class="relative size-full overflow-hidden bg-black">
  {#each pictures as { clip, active } (clipKey(clip))}
    <div class="absolute inset-0" style="z-index: {clip.track}; visibility: {active ? 'visible' : 'hidden'}">
      {#if playable(clip) && clip.kind === "image"}
        <img src={src(clip)} alt="" class="size-full object-contain" style={lookStyle(clip.look, timeline.width, timeline.height)} onerror={() => fail(clip)} />
      {:else if playable(clip) && clip.kind === "video"}
        <video
          src={src(clip)}
          muted
          playsinline
          preload="auto"
          class="size-full object-contain"
          style={lookStyle(clip.look, timeline.width, timeline.height)}
          use:sync={{ clip, active, time, playing }}
          onerror={() => fail(clip)}
        ></video>
      {:else if active}
        <div class="flex size-full flex-col items-center justify-center gap-1.5 bg-neutral-900 px-6 text-center text-white/70">
          <ImageOff class="size-5" />
          <p class="max-w-full truncate text-xs font-medium">{clip.name || "Untitled clip"}</p>
          <p class="text-2xs text-white/50">
            {clip.kind === "missing" || !clip.media
              ? "Media offline, or a title or generator."
              : `${extension(clip.name)} can't play in the browser. Render to see it.`}
          </p>
        </div>
      {/if}
    </div>
  {/each}
  {#each sounds as { clip, active } (`a${clipKey(clip)}`)}
    <audio src={src(clip)} preload="auto" use:sync={{ clip, active, time, playing, muted, volume }} onerror={() => fail(clip)}></audio>
  {/each}
</div>
