<script lang="ts">
  // Mounted only while a turn runs; ticks once a second, never animates.
  const startedAt = Date.now();
  let seconds = $state(0);

  $effect(() => {
    const timer = setInterval(() => (seconds = Math.floor((Date.now() - startedAt) / 1000)), 1000);
    return () => clearInterval(timer);
  });

  const elapsed = $derived(seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}m ${seconds % 60}s`);
</script>

<div class="flex items-center gap-2 px-1 text-sm text-muted-foreground">
  <span class="size-1.5 rounded-full bg-primary"></span>
  Working for {elapsed}
</div>
