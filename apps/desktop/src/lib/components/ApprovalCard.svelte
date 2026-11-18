<script lang="ts">
  import GraduationCap from "@lucide/svelte/icons/graduation-cap";
  import ShieldAlert from "@lucide/svelte/icons/shield-alert";
  import type { ApprovalRequest } from "#shared/protocol.ts";
  import { approvalDetail, briefRequest, isResolveTool, lineDiff, toolLabel } from "#lib/labels.ts";

  let { request, onAnswer }: { request: ApprovalRequest; onAnswer: (allow: boolean) => void } = $props();

  // A change to the brief reads as what it is: new rules against the old, or a lesson next to the user's words.
  const brief = $derived(briefRequest(request.tool, request.input));
  const diff = $derived(brief?.kind === "rules" ? lineDiff(brief.was, brief.rules) : []);
  const LINE = { same: "text-muted-foreground", added: "bg-success/10 text-foreground", removed: "bg-destructive/10 text-muted-foreground line-through" };
  const MARK = { same: " ", added: "+", removed: "−" };
</script>

<div class="rounded-2xl border border-warning/30 bg-card px-4 py-3 shadow-composer">
  {#if brief}
    <div class="flex items-center gap-2 text-sm">
      <GraduationCap class="size-4 shrink-0 text-warning" />
      <span class="min-w-0 flex-1 truncate">
        <span class="font-medium">{brief.kind === "lesson" ? "New lesson for" : brief.was ? "Change the rules of" : "Rules for"}</span>
        {brief.for}
      </span>
    </div>
    {#if brief.kind === "lesson"}
      <div class="mt-2 ms-6 flex flex-col gap-2 text-sm select-text">
        <div>
          <p class="text-2xs text-muted-foreground">From what you said</p>
          <p class="border-s-2 ps-2 text-muted-foreground italic">{brief.feedback}</p>
        </div>
        <div>
          <p class="text-2xs text-muted-foreground">The agent will follow this from now on</p>
          <p>{brief.rule}</p>
        </div>
      </div>
    {:else}
      {#if brief.summary}
        <p class="mt-2 ms-6 text-sm text-muted-foreground select-text">{brief.summary}</p>
      {/if}
      <div class="mt-2 ms-6 max-h-72 overflow-auto rounded-md border font-mono text-xs leading-5 select-text" aria-label="The rules, with what changes">
        {#each diff as line, index (index)}
          <div class="flex gap-2 px-2 whitespace-pre-wrap {LINE[line.kind]}"><span class="shrink-0 select-none">{MARK[line.kind]}</span>{line.text || " "}</div>
        {/each}
      </div>
    {/if}
    <p class="mt-2 ms-6 text-2xs text-muted-foreground">
      {brief.kind === "lesson"
        ? "Decline it if this isn't what you meant. You can remove a lesson later in Settings."
        : "Every edit in it follows these. You can change them later in Settings."}
    </p>
  {:else}
    <div class="flex items-center gap-2 text-sm">
      <ShieldAlert class="size-4 shrink-0 text-warning" />
      <span class="min-w-0 flex-1 truncate">
        <span class="font-medium">Allow {isResolveTool(request.tool) ? "Resolve" : "tool"}:</span>
        <span class="font-mono text-xs text-foreground/80">{toolLabel(request.tool, request.input)}</span>
      </span>
    </div>
    <pre
      class="mt-2 ms-6 max-h-64 overflow-auto font-mono text-xs leading-5 whitespace-pre-wrap text-muted-foreground select-text">{approvalDetail(
        request.tool,
        request.input,
      )}</pre>
  {/if}
  <div class="mt-3 flex justify-end gap-2">
    <button
      type="button"
      class="h-7 rounded-md border px-3 text-xs font-medium transition-colors hover:bg-accent"
      onclick={() => onAnswer(false)}
    >
      {brief ? "Decline" : "Deny"}
    </button>
    <button
      type="button"
      class="h-7 rounded-md bg-primary px-3 text-xs font-medium text-primary-foreground transition-opacity hover:opacity-90"
      onclick={() => onAnswer(true)}
    >
      {brief ? "Accept" : "Allow"}
    </button>
  </div>
</div>
