<script lang="ts" module>
  export type ToolIconKind = "resolve" | "terminal" | "read" | "edit" | "search" | "web" | "todo" | "schema" | "tool" | "chevron" | "x";
</script>

<script lang="ts">
  // Lucide's icons (ISC), drawn as plain SVG: a long chat has hundreds of tool rows, and Lucide's component builds
  // each path at runtime, which made opening a long thread twice as slow.
  let { kind, class: className = "" }: { kind: ToolIconKind; class?: string } = $props();
</script>

<svg
  xmlns="http://www.w3.org/2000/svg"
  width="24"
  height="24"
  viewBox="0 0 24 24"
  fill="none"
  stroke="currentColor"
  stroke-width="2"
  stroke-linecap="round"
  stroke-linejoin="round"
  class={className}
  aria-hidden="true"
>
  {#if kind === "resolve"}
    <path d="m12.296 3.464 3.02 3.956" /><path d="M20.2 6 3 11l-.9-2.4c-.3-1.1.3-2.2 1.3-2.5l13.5-4c1.1-.3 2.2.3 2.5 1.3z" /><path
      d="M3 11h18v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"
    /><path d="m6.18 5.276 3.1 3.899" />
  {:else if kind === "terminal"}
    <path d="m7 11 2-2-2-2" /><path d="M11 13h4" /><rect width="18" height="18" x="3" y="3" rx="2" ry="2" />
  {:else if kind === "read"}
    <path d="M6 22a2 2 0 0 1-2-2V4a2 2 0 0 1 2-2h8a2.4 2.4 0 0 1 1.704.706l3.588 3.588A2.4 2.4 0 0 1 20 8v12a2 2 0 0 1-2 2z" /><path
      d="M14 2v5a1 1 0 0 0 1 1h5"
    /><path d="M10 9H8" /><path d="M16 13H8" /><path d="M16 17H8" />
  {:else if kind === "edit"}
    <path d="M12.659 22H18a2 2 0 0 0 2-2V8a2.4 2.4 0 0 0-.706-1.706l-3.588-3.588A2.4 2.4 0 0 0 14 2H6a2 2 0 0 0-2 2v9.34" /><path
      d="M14 2v5a1 1 0 0 0 1 1h5"
    /><path
      d="M10.378 12.622a1 1 0 0 1 3 3.003L8.36 20.637a2 2 0 0 1-.854.506l-2.867.837a.5.5 0 0 1-.62-.62l.836-2.869a2 2 0 0 1 .506-.853z"
    />
  {:else if kind === "search"}
    <path d="m21 21-4.34-4.34" /><circle cx="11" cy="11" r="8" />
  {:else if kind === "web"}
    <circle cx="12" cy="12" r="10" /><path d="M12 2a14.5 14.5 0 0 0 0 20 14.5 14.5 0 0 0 0-20" /><path d="M2 12h20" />
  {:else if kind === "todo"}
    <path d="M13 5h8" /><path d="M13 12h8" /><path d="M13 19h8" /><path d="m3 17 2 2 4-4" /><rect x="3" y="4" width="6" height="6" rx="1" />
  {:else if kind === "schema"}
    <path d="M12 5v16" /><path
      d="M20.001 19A2 2 0 0022 17V5a2 2 0 00-1.999-2L16 3.002A5 5 0 0012 5a5 5 0 00-4-2H4a2 2 0 00-2 2v12a2 2 0 001.999 2H8a5 5 0 014 2 5 5 0 014-2z"
    />
  {:else if kind === "chevron"}
    <path d="m9 18 6-6-6-6" />
  {:else if kind === "x"}
    <path d="M18 6 6 18" /><path d="m6 6 12 12" />
  {:else}
    <path
      d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.106-3.105c.32-.322.863-.22.983.218a6 6 0 0 1-8.259 7.057l-7.91 7.91a1 1 0 0 1-2.999-3l7.91-7.91a6 6 0 0 1 7.057-8.259c.438.12.54.662.219.984z"
    />
  {/if}
</svg>
