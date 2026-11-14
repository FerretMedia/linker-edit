<script lang="ts">
  let { label, checked, onChange }: { label: string; checked: boolean; onChange: (checked: boolean) => void } = $props();
</script>

<button
  type="button"
  role="switch"
  aria-label={label}
  aria-checked={checked}
  class="relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors {checked ? 'bg-primary' : 'bg-input'}"
  onclick={() => onChange(!checked)}
>
  <span class="inline-block size-4 rounded-full bg-white shadow-sm transition-transform {checked ? 'translate-x-4.5' : 'translate-x-0.5'}"></span>
</button>
