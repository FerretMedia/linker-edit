<script lang="ts">
  import { untrack } from "svelte";
  import { Clapperboard, CircleAlert, History, LoaderCircle, MonitorPlay, RefreshCw, Scissors, X } from "@lucide/svelte";
  import type { PlayableTimeline, PreviewState, ResolveStatus } from "#shared/protocol.ts";
  import PlayerControls from "#lib/components/PlayerControls.svelte";
  import TimelinePlayer from "#lib/components/TimelinePlayer.svelte";
  import { nearestCut, type PreviewTarget } from "#lib/player.ts";

  let {
    projectId,
    target,
    preview,
    resolve,
    busy,
    onTarget,
    onRender,
    onStop,
    onClose,
  }: {
    /** The project whose edit history earlier versions come from. */
    projectId: string | null;
    target: PreviewTarget;
    preview: PreviewState;
    resolve: ResolveStatus;
    busy: boolean;
    onTarget: (target: PreviewTarget) => void;
    onRender: (timelineId: string) => void;
    onStop: () => void;
    onClose: () => void;
  } = $props();

  const MODES = [
    { value: "cut", label: "Cut preview", detail: "Instant, no color or effects", Icon: Scissors },
    { value: "render", label: "Full render", detail: "Exactly as Resolve plays it", Icon: Clapperboard },
  ] as const;

  let timeline = $state<PlayableTimeline | null>(null);
  let loading = $state(false);
  let loadError = $state<string | null>(null);
  let reloads = $state(0);
  let chosen = $state<"cut" | "render">("cut");
  let time = $state(0);
  let playing = $state(false);
  let muted = $state(false);
  let volume = $state(1);
  let fullscreen = $state(false);
  let stage: HTMLElement | undefined = $state();
  let video: HTMLVideoElement | undefined = $state();

  const rendered = $derived(preview.state === "ready" && timeline && preview.timeline.id === timeline.id ? preview : null);
  const renderingThis = $derived(preview.state === "rendering");
  // Rendered only while a render of this timeline exists: a new render deletes the last one.
  const source = $derived(chosen === "render" && rendered ? "render" : "cut");
  const duration = $derived(source === "render" && video?.duration ? video.duration : (timeline?.duration ?? 0));
  const renderBlocked = $derived.by(() => {
    if (target.commit) return "Only the timeline as it is in Resolve now can be rendered. Restore this version first.";
    if (resolve.state !== "connected") return "Connect Resolve to render.";
    if (busy) return "Wait for the agent to finish.";
    if (!timeline) return "Load a timeline first.";
    return null;
  });

  $effect(() => {
    // Loads the timeline's clip list from the server whenever the target changes or a reload is asked for.
    void reloads;
    const params = new URLSearchParams();
    if (projectId) params.set("project", projectId);
    if (target.id) params.set("id", target.id);
    if (target.commit) params.set("commit", target.commit);
    const controller = new AbortController();
    loading = true;
    loadError = null;
    fetch(`/api/timeline?${params}`, { signal: controller.signal })
      .then(async (response) => {
        const body = (await response.json()) as PlayableTimeline | { error: string };
        if ("error" in body) throw new Error(body.error);
        if (timeline?.id !== body.id || timeline.commit !== body.commit) {
          time = 0;
          playing = false;
          chosen = "cut";
        }
        timeline = body;
      })
      .catch((error: unknown) => {
        if (!controller.signal.aborted) loadError = error instanceof Error ? error.message : String(error);
      })
      .finally(() => {
        if (!controller.signal.aborted) loading = false;
      });
    return () => controller.abort();
  });

  let wasBusy = false;
  $effect(() => {
    // The agent may have changed the live timeline: reload it once its turn ends.
    if (wasBusy && !busy && !target.commit) reloads++;
    wasBusy = busy;
  });

  // A render that was already there when the panel opened doesn't take over; one finishing now does.
  let shownRender: string | null = untrack(() => (preview.state === "ready" ? preview.url : null));
  $effect(() => {
    // A render that just finished takes over at the same position.
    if (rendered && rendered.url !== shownRender) {
      shownRender = rendered.url;
      chosen = "render";
    }
  });

  function seek(next: number) {
    time = Math.min(duration, Math.max(0, next));
  }

  function togglePlay() {
    if (!playing && time >= duration - 0.01) time = 0;
    playing = !playing;
  }

  function toggleFullscreen() {
    if (document.fullscreenElement) void document.exitFullscreen();
    else void stage?.requestFullscreen();
  }

  function onKey(event: KeyboardEvent) {
    if (!timeline) return;
    const frame = 1 / timeline.fps;
    const keys: Record<string, () => void> = {
      " ": togglePlay,
      k: togglePlay,
      ArrowLeft: () => seek(time - (event.shiftKey ? 1 : frame)),
      ArrowRight: () => seek(time + (event.shiftKey ? 1 : frame)),
      ArrowUp: () => seek(nearestCut(timeline!, time, -1)),
      ArrowDown: () => seek(nearestCut(timeline!, time, 1)),
      Home: () => seek(0),
      End: () => seek(duration),
      f: toggleFullscreen,
      m: () => (muted = !muted),
    };
    const action = keys[event.key];
    if (!action) return;
    event.preventDefault();
    action();
  }
</script>

<svelte:document onfullscreenchange={() => (fullscreen = document.fullscreenElement === stage)} />

<aside class="flex w-[32rem] shrink-0 flex-col border-l bg-sidebar">
  <header class="title-bar flex h-13 shrink-0 items-center gap-2 border-b px-4">
    <MonitorPlay class="size-4 text-muted-foreground" />
    <h2 class="text-sm font-medium">Preview</h2>
    <button
      type="button"
      class="ml-auto flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      onclick={onClose}
      title="Close preview"
    >
      <X class="size-4" />
    </button>
  </header>

  <div class="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-4">
    <div class="flex min-w-0 items-center gap-2">
      <div class="min-w-0 flex-1">
        <p class="truncate text-sm font-medium" title={timeline?.name}>{timeline?.name ?? (loading ? "Loading timeline…" : "No timeline")}</p>
        <p class="flex items-center gap-1 text-2xs text-muted-foreground">
          {#if target.commit}
            <History class="size-3" />
            Version from save <span class="font-mono">{target.commit}</span> ·
            <button type="button" class="underline-offset-2 hover:text-foreground hover:underline" onclick={() => onTarget({ id: target.id, commit: null })}>
              show it as it is now
            </button>
          {:else}
            As it is in Resolve now ·
            <button type="button" class="underline-offset-2 hover:text-foreground hover:underline" onclick={() => reloads++}>reload</button>
          {/if}
        </p>
      </div>
      {#if renderingThis && preview.state === "rendering"}
        <div class="flex h-8 shrink-0 items-center gap-2 rounded-lg border bg-card ps-2.5 pe-1 text-xs">
          <LoaderCircle class="size-3.5 animate-spin text-primary" />
          <span class="tabular-nums">Rendering {preview.percent}%</span>
          <button type="button" class="flex size-6 items-center justify-center rounded-md text-muted-foreground hover:bg-accent hover:text-foreground" title="Stop rendering" onclick={onStop}>
            <X class="size-3.5" />
          </button>
        </div>
      {:else}
        <button
          type="button"
          class="flex h-8 shrink-0 items-center gap-1.5 rounded-lg bg-primary px-3 text-xs font-medium text-primary-foreground transition-opacity enabled:hover:opacity-90 disabled:opacity-40"
          disabled={renderBlocked !== null}
          title={renderBlocked ?? "Have Resolve render the timeline with color, effects and transitions"}
          onclick={() => timeline && onRender(timeline.id)}
        >
          {#if rendered}<RefreshCw class="size-3.5" /> Render again{:else}<Clapperboard class="size-3.5" /> Full render{/if}
        </button>
      {/if}
    </div>

    <div class="grid grid-cols-2 gap-0.5 rounded-lg border bg-card p-0.5">
      {#each MODES as mode (mode.value)}
        {@const unavailable = mode.value === "render" && !rendered}
        <button
          type="button"
          class="flex items-start gap-2 rounded-md px-2.5 py-1.5 text-left transition-colors {source === mode.value
            ? 'bg-accent text-foreground'
            : 'text-muted-foreground enabled:hover:text-foreground'} disabled:opacity-50"
          disabled={unavailable}
          aria-pressed={source === mode.value}
          title={unavailable ? "Use Full render to watch the timeline exactly as Resolve plays it" : undefined}
          onclick={() => (chosen = mode.value)}
        >
          <mode.Icon class="mt-0.5 size-3.5 shrink-0" />
          <span class="min-w-0">
            <span class="block text-xs font-medium">{mode.label}</span>
            <span class="block truncate text-2xs text-muted-foreground">
              {unavailable ? "Not rendered yet" : mode.detail}
            </span>
          </span>
        </button>
      {/each}
    </div>

    <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
    <div
      bind:this={stage}
      class="flex flex-col overflow-hidden rounded-xl bg-neutral-950 outline-none focus-visible:ring-2 focus-visible:ring-primary/70 {fullscreen ? 'justify-center' : ''}"
      tabindex="0"
      role="region"
      aria-label="Video preview"
      onkeydown={onKey}
    >
      <div
        class="relative w-full {fullscreen ? 'min-h-0 flex-1' : ''}"
        style={fullscreen ? "" : `aspect-ratio: ${timeline ? `${timeline.width} / ${timeline.height}` : "16 / 9"}`}
      >
        {#if timeline}
          <span
            class="pointer-events-none absolute top-2 left-2 z-20 flex items-center gap-1 rounded-md bg-black/65 px-1.5 py-0.5 text-[0.625rem] font-medium tracking-wide text-white uppercase"
          >
            {#if source === "render"}<Clapperboard class="size-3" /> Full render{:else}<Scissors class="size-3" /> Cut preview{/if}
          </span>
        {/if}
        {#if source === "render" && rendered}
          <!-- svelte-ignore a11y_media_has_caption -->
          <video
            bind:this={video}
            src={rendered.url}
            bind:currentTime={time}
            bind:paused={() => !playing, (paused) => (playing = !paused)}
            {muted}
            bind:volume
            preload="auto"
            class="size-full bg-black object-contain"
            onclick={togglePlay}
          ></video>
        {:else if timeline}
          <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
          <div class="size-full" onclick={togglePlay}>
            <TimelinePlayer {timeline} bind:time bind:playing {muted} {volume} />
          </div>
        {:else}
          <div class="flex size-full items-center justify-center text-xs text-white/60">
            {#if loading}
              <LoaderCircle class="size-5 animate-spin" />
            {:else if loadError}
              <p class="flex items-start gap-2 px-6 text-center"><CircleAlert class="mt-px size-3.5 shrink-0" />{loadError}</p>
            {:else}
              Open a timeline in Resolve to preview it.
            {/if}
          </div>
        {/if}
      </div>
      <PlayerControls
        {timeline}
        {duration}
        {time}
        {playing}
        {muted}
        {volume}
        {fullscreen}
        onSeek={seek}
        onTogglePlay={togglePlay}
        onCut={(direction) => timeline && seek(nearestCut(timeline, time, direction))}
        onToggleMute={() => (muted = !muted)}
        onVolume={(next) => ((volume = next), (muted = next === 0))}
        onToggleFullscreen={toggleFullscreen}
      />
    </div>

    {#if source === "render" && rendered?.outdated}
      <p class="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-xs text-warning-foreground">
        Resolve changed after this render. Cut preview shows the latest cut; render again for the exact picture.
      </p>
    {/if}
    {#if preview.state === "error"}
      <p class="flex items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/8 px-3 py-2 text-xs text-destructive">
        <CircleAlert class="mt-px size-3.5 shrink-0" />
        {preview.message}
      </p>
    {/if}

    <p class="mt-auto text-2xs leading-relaxed text-muted-foreground">
      <span class="font-medium text-foreground/80">Cut preview</span> plays your media straight from disk, right away and
      for any saved version: the cuts, zoom, position and opacity, but no color, effects, transitions or titles.
      <span class="font-medium text-foreground/80">Full render</span> has Resolve render the exact picture as a small MP4;
      Resolve switches to the timeline while it renders and puts everything back after. Keys: Space play, ← → frame, ↑ ↓
      cut, F full screen.
    </p>
  </div>
</aside>
