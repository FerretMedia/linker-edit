<script lang="ts" generics="T extends string">
  // A small choice of a few options, all in view.
  let {
    label,
    options,
    value,
    onChange,
  }: { label: string; options: readonly { value: T; label: string; title?: string }[]; value: T; onChange: (value: T) => void } = $props();
</script>

<div class="inline-flex rounded-lg border bg-muted p-0.5" role="radiogroup" aria-label={label}>
  {#each options as option (option.value)}
    <button
      type="button"
      role="radio"
      aria-checked={option.value === value}
      title={option.title}
      class="h-7 rounded-md px-3 text-xs transition-colors {option.value === value
        ? 'bg-card text-foreground shadow-sm shadow-black/10 ring-1 ring-border'
        : 'text-muted-foreground hover:text-foreground'}"
      onclick={() => option.value !== value && onChange(option.value)}
    >
      {option.label}
    </button>
  {/each}
</div>
