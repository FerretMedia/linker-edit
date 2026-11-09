<script lang="ts">
  import { Clapperboard, Film, RefreshCw, SquarePen } from "@lucide/svelte";
  import type { ResolveStatus } from "#shared/protocol.ts";

  let {
    resolve,
    version,
    update,
    chatTitle,
    busy,
    onNewChat,
    onRefreshResolve,
  }: {
    resolve: ResolveStatus;
    version: string;
    update: string | null;
    chatTitle: string;
    busy: boolean;
    onNewChat: () => void;
    onRefreshResolve: () => void;
  } = $props();

  const status = $derived.by(() => {
    switch (resolve.state) {
      case "checking":
        return { tone: "bg-warning", title: "Checking Resolve…", detail: "" };
      case "connected":
        return {
          tone: "bg-success",
          title: `${resolve.product} ${resolve.version.split(".").slice(0, 3).join(".")}`,
          detail: resolve.timeline ? `Timeline: ${resolve.timeline}` : "No timeline open",
        };
      case "not_running":
        return { tone: "bg-muted-foreground", title: "Resolve is not running", detail: "Start Resolve, then refresh." };
      case "blocked":
        return { tone: "bg-warning", title: "Resolve needs attention", detail: resolve.hint };
      case "unreachable":
        return { tone: "bg-destructive", title: "Resolve not reachable", detail: resolve.hint };
      case "setup_needed":
        return { tone: "bg-destructive", title: "Setup needed", detail: resolve.hint };
    }
  });

  const projectName = $derived(resolve.state === "connected" ? (resolve.project ?? "No project open") : "Resolve");
</script>

<aside class="flex w-64 shrink-0 flex-col border-r bg-sidebar">
  <div class="flex h-13 items-center gap-2 px-4">
    <span class="flex size-6 items-center justify-center rounded-md bg-primary text-primary-foreground">
      <Clapperboard class="size-3.5" strokeWidth={2.25} />
    </span>
    <span class="text-sm font-semibold tracking-tight">Linker-Edit</span>
    <span class="ml-auto text-2xs text-muted-foreground">{version === "0.0.0" ? "dev" : version}</span>
  </div>

  <nav class="flex flex-1 flex-col gap-0.5 overflow-y-auto px-2">
    <button
      type="button"
      class="flex h-8 items-center gap-2 rounded-lg px-2.5 text-sm text-muted-foreground transition-colors hover:bg-accent hover:text-foreground"
      onclick={onNewChat}
    >
      <SquarePen class="size-4" />
      New chat
    </button>

    <div class="mt-4 flex h-7 items-center gap-2 px-2.5 text-xs font-medium text-muted-foreground">
      <Film class="size-3.5" />
      <span class="truncate" title={projectName}>{projectName}</span>
    </div>
    <div
      class="flex h-8 items-center gap-2 rounded-lg bg-sidebar-row-active px-2.5 text-sm shadow-[0_0_0_1px_var(--border)]"
      aria-current="page"
    >
      <span class="min-w-0 flex-1 truncate">{chatTitle}</span>
      {#if busy}
        <span class="size-1.5 shrink-0 rounded-full bg-primary" title="Working"></span>
      {/if}
    </div>
  </nav>

  {#if update}
    <div class="mx-2 rounded-lg px-2.5 py-2 shadow-[0_0_0_1px_var(--border)]">
      <span class="block text-xs font-medium">Linker-Edit {update} is out</span>
      <span class="mt-0.5 block text-2xs text-muted-foreground">
        Stop this one, then run <code class="font-mono">npx linker-edit@latest</code>.
      </span>
    </div>
  {/if}

  <button
    type="button"
    class="group m-2 flex items-start gap-2.5 rounded-lg px-2.5 py-2 text-left transition-colors hover:bg-accent"
    onclick={onRefreshResolve}
    title="Refresh Resolve status"
  >
    <span class="mt-1.5 size-2 shrink-0 rounded-full {status.tone}"></span>
    <span class="min-w-0 flex-1">
      <span class="block truncate text-xs font-medium">{status.title}</span>
      {#if status.detail}
        <span class="mt-0.5 block text-2xs text-muted-foreground">{status.detail}</span>
      {/if}
    </span>
    <RefreshCw class="mt-0.5 size-3.5 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100" />
  </button>
</aside>
