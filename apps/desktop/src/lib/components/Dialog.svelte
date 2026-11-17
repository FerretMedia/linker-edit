<script lang="ts">
  import type { Snippet } from "svelte";
  import { X } from "@lucide/svelte";

  let { title, onClose, children }: { title: string; onClose: () => void; children: Snippet } = $props();
  let dialog: HTMLDialogElement;

  $effect(() => {
    // The native dialog traps focus and closes on Escape; it has to be opened from script to be modal.
    dialog.showModal();
  });
</script>

<dialog
  bind:this={dialog}
  class="m-auto w-[30rem] max-w-[calc(100vw-2rem)] rounded-xl border bg-card p-0 text-foreground shadow-xl shadow-black/20 backdrop:bg-black/40"
  onclose={onClose}
  onclick={(event) => event.target === dialog && dialog.close()}
>
  <header class="flex h-12 items-center gap-2 border-b px-4">
    <h2 class="text-sm font-medium">{title}</h2>
    <button
      type="button"
      class="ml-auto flex size-7 items-center justify-center rounded-md text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      aria-label="Close"
      onclick={() => dialog.close()}
    >
      <X class="size-4" />
    </button>
  </header>
  <div class="flex flex-col gap-4 p-4">
    {@render children()}
  </div>
</dialog>
