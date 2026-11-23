<script lang="ts">
  // T3 Code's sidebar thread row (apps/web/src/components/sidebar/SidebarThreadRow.tsx; MIT, see NOTICE):
  // "card" for pinned, active and working threads, "folder" inside a project folder, "slim" in the Snoozed and Settled
  // shelves.
  import AlarmClock from "@lucide/svelte/icons/alarm-clock";
  import AlarmClockOff from "@lucide/svelte/icons/alarm-clock-off";
  import Check from "@lucide/svelte/icons/check";
  import CircleCheck from "@lucide/svelte/icons/circle-check";
  import CircleDashed from "@lucide/svelte/icons/circle-dashed";
  import Clock from "@lucide/svelte/icons/clock";
  import Pin from "@lucide/svelte/icons/pin";
  import PinOff from "@lucide/svelte/icons/pin-off";
  import ShieldQuestion from "@lucide/svelte/icons/shield-question-mark";
  import Monitor from "@lucide/svelte/icons/monitor";
  import Undo2 from "@lucide/svelte/icons/undo-2";
  import {
    activityTimestamp,
    relativeTime,
    settledTimestamp,
    shouldRecede,
    snoozeWakeLabel,
    topStatus,
    workingDuration,
    type SidebarThread,
  } from "#lib/sidebar.ts";
  import ProjectIcon from "./ProjectIcon.svelte";

  let {
    thread,
    variant,
    action,
    machine = null,
    active,
    nowMs,
    renaming,
    jumpLabel = null,
    onSelect,
    onStartRename,
    onRename,
    onSettle,
    onUnsettle,
    onUnsnooze,
    onUnpin,
    onSnoozeMenu,
    onAcknowledgeWoke,
    onContextMenu,
  }: {
    thread: SidebarThread;
    variant: "card" | "folder" | "slim";
    action: "settle" | "unsettle" | "unsnooze";
    /** The machine it's on, when there's more than one. */
    machine?: string | null;
    active: boolean;
    nowMs: number;
    renaming: boolean;
    jumpLabel?: string | null;
    onSelect: () => void;
    onStartRename: () => void;
    /** Null cancels. */
    onRename: (title: string | null) => void;
    onSettle: () => void;
    onUnsettle: () => void;
    onUnsnooze: () => void;
    onUnpin: () => void;
    onSnoozeMenu: (x: number, y: number) => void;
    onAcknowledgeWoke: () => void;
    onContextMenu: (x: number, y: number) => void;
  } = $props();

  const STATUS = {
    working: { label: "Working", Icon: CircleDashed, class: "text-info-foreground" },
    approval: { label: "Approval", Icon: ShieldQuestion, class: "text-warning-foreground" },
    woke: { label: "Woke", Icon: AlarmClock, class: "text-warning-foreground" },
    done: { label: "Done", Icon: CircleCheck, class: "text-success-foreground" },
  };

  const status = $derived(topStatus(thread, nowMs));
  const top = $derived(status ? STATUS[status] : null);
  const recede = $derived(shouldRecede(thread, nowMs, active));
  const pinned = $derived(!!thread.pinnedAt);
  const time = $derived(relativeTime(action === "unsettle" ? settledTimestamp(thread) : activityTimestamp(thread), nowMs));

  // How long the turn has run, after "Working", ticking each second like T3 Code's. It started with the user's message.
  const startedMs = $derived(thread.lastSentAt ? Date.parse(thread.lastSentAt) : null);
  let secondNow = $state(Date.now());
  $effect(() => {
    if (status !== "working" || variant !== "card") return;
    secondNow = Date.now();
    const timer = setInterval(() => (secondNow = Date.now()), 1000);
    return () => clearInterval(timer);
  });

  let draft = $state("");
  let done = false;

  function startEditing(node: HTMLInputElement) {
    draft = thread.title;
    done = false;
    node.focus();
    node.select();
  }

  function finish(save: boolean) {
    if (done) return;
    done = true;
    onRename(save && draft.trim() ? draft.trim() : null);
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.target !== event.currentTarget) return;
    if (event.key === "F2" && !renaming) {
      event.preventDefault();
      onStartRename();
    } else if (event.key === "Enter" || event.key === " ") {
      event.preventDefault();
      onSelect();
    }
  }

  function onDblclick(event: MouseEvent) {
    if (renaming || event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;
    if ((event.target as HTMLElement).closest("button, a, input")) return;
    event.preventDefault();
    onStartRename();
  }

  const stop = (handler: () => void) => (event: MouseEvent) => {
    event.preventDefault();
    event.stopPropagation();
    handler();
  };

  const surface = $derived(
    [
      "group/sidebar-row relative w-full cursor-pointer overflow-hidden rounded-md text-left outline-none select-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring",
      action === "unsettle" && !active ? "[&:not(:hover):not(:focus-within)_*]:text-secondary-label" : "",
      active
        ? "bg-sidebar-row-active text-sidebar-foreground light:ring-1 light:ring-inset light:ring-sidebar-border/70"
        : recede
          ? "text-sidebar-muted-foreground/75 hover:bg-sidebar-row-hover hover:text-sidebar-foreground"
          : "bg-transparent text-sidebar-foreground hover:bg-sidebar-row-hover",
      recede && status === "working" ? "opacity-70 transition-opacity hover:opacity-100 focus-within:opacity-100 motion-reduce:transition-none" : "",
      // Its machine is offline: shown as it was, and it doesn't open.
      thread.project.offline ? "cursor-default opacity-60" : "",
    ].join(" "),
  );

  const titleClass = $derived(
    [
      "min-w-0 flex-1 truncate text-start text-sm transition-opacity motion-reduce:transition-none",
      recede ? "font-normal" : "font-medium",
      variant !== "slim"
        ? recede
          ? "text-foreground/75 group-hover/sidebar-row:text-foreground group-focus-visible/sidebar-row:text-foreground"
          : "text-foreground"
        : [
            "group-focus-within/sidebar-row:text-foreground group-hover/sidebar-row:text-foreground",
            recede ? "text-secondary-label" : active || status === "woke" ? "text-foreground" : status === "done" ? "text-muted-foreground" : "text-secondary-label/70",
          ].join(" "),
    ].join(" "),
  );

  const label = $derived([thread.title, top?.label, thread.project.name, machine && `on ${machine}`, thread.project.offline && "offline"].filter(Boolean).join(", "));
  const tooltip = $derived([thread.title, machine ? `${thread.project.name} · ${machine}` : thread.project.name].join("\n"));
</script>

{#snippet title()}
  {#if renaming}
    <input
      value={thread.title}
      dir="auto"
      aria-label="Thread title"
      use:startEditing
      oninput={(event) => (draft = event.currentTarget.value)}
      onkeydown={(event) => {
        event.stopPropagation();
        if (event.isComposing) return;
        if (event.key === "Enter") {
          event.preventDefault();
          finish(true);
        } else if (event.key === "Escape") {
          event.preventDefault();
          finish(false);
        }
      }}
      onblur={() => finish(true)}
      onclick={(event) => event.stopPropagation()}
      ondblclick={(event) => event.stopPropagation()}
      class="-mx-1 min-w-0 flex-1 rounded-sm bg-card px-1 text-sm font-medium text-foreground ring-1 ring-input outline-none ring-inset focus:ring-foreground"
    />
  {:else}
    <span class={titleClass} dir="auto">{thread.title}</span>
  {/if}
{/snippet}

{#snippet pinIndicator()}
  {#if pinned}
    <button
      type="button"
      aria-label="Unpin thread"
      title="Unpin thread"
      onclick={stop(onUnpin)}
      class="group/unpin inline-flex cursor-pointer items-center rounded-sm text-muted-foreground/65 outline-none transition-colors hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
    >
      <Pin aria-hidden="true" class="size-3 shrink-0 group-hover/unpin:hidden group-focus-visible/unpin:hidden" />
      <PinOff aria-hidden="true" class="hidden size-3 shrink-0 group-hover/unpin:block group-focus-visible/unpin:block" />
    </button>
  {/if}
{/snippet}

{#snippet jumpHint()}
  {#if jumpLabel}
    <kbd
      aria-hidden="true"
      data-jump-hint
      class="pointer-events-none inline-flex h-5 min-w-5 items-center justify-center gap-1 rounded bg-foreground/[0.07] px-1 font-sans text-xs font-medium text-muted-foreground select-none"
    >
      {jumpLabel}
    </kbd>
  {/if}
{/snippet}

{#if variant !== "card"}
  {@const inFolder = variant === "folder"}
  <li class="list-none [contain-intrinsic-size:auto_--spacing(9)] [content-visibility:auto]">
    <div
      role="button"
      tabindex="0"
      data-testid={inFolder ? "sidebar-row-folder" : "sidebar-row-slim"}
      data-thread-id={thread.id}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      title={tooltip}
      class="{surface} flex h-9 items-center gap-2.5 px-2.5 {inFolder ? 'ps-9' : ''}"
      onclick={onSelect}
      ondblclick={onDblclick}
      onkeydown={onKeydown}
      oncontextmenu={(event) => {
        event.preventDefault();
        onContextMenu(event.clientX, event.clientY);
      }}
    >
      {#if !inFolder}
        <span
          class="flex shrink-0 items-center transition-opacity {!active || action === 'unsettle'
            ? 'opacity-40 grayscale group-focus-within/sidebar-row:opacity-100 group-focus-within/sidebar-row:grayscale-0 group-hover/sidebar-row:opacity-100 group-hover/sidebar-row:grayscale-0'
            : ''}"
        >
          <ProjectIcon icon={thread.project.icon} name={thread.project.name} />
        </span>
      {/if}
      {@render title()}
      {#if machine && !inFolder}
        <span class="max-w-20 shrink-0 truncate text-2xs text-sidebar-muted-foreground">{machine}</span>
      {/if}
      {@render pinIndicator()}
      <span
        class="relative ml-auto flex h-6 min-w-8 shrink-0 items-center justify-end {jumpLabel
          ? '[&>:not([data-jump-hint])]:invisible [&>:not([data-jump-hint])]:absolute'
          : ''}"
      >
        <span class="inline-flex justify-end text-secondary-label tabular-nums transition-opacity {status === 'woke' ? '' : 'group-hover/sidebar-row:opacity-0'}">
          {#if action === "unsnooze" && thread.snoozedUntil}
            <span class="inline-flex items-center gap-1 text-xs text-info-foreground tabular-nums">
              <AlarmClock aria-hidden="true" class="size-3" />
              {snoozeWakeLabel(thread.snoozedUntil, nowMs)}
            </span>
          {:else if status === "woke"}
            <button
              type="button"
              aria-label="Dismiss Woke notification"
              title="Dismiss Woke notification"
              onclick={stop(onAcknowledgeWoke)}
              class="inline-flex cursor-pointer items-center gap-1 rounded-sm text-xs font-medium text-warning-foreground outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
            >
              <AlarmClock aria-hidden="true" class="size-3" />
              <span role="status">Woke</span>
            </button>
          {:else if inFolder && top}
            <span class="inline-flex items-center gap-1 text-xs font-medium {top.class}">
              <top.Icon class="size-3 shrink-0" />
              {top.label}
            </span>
          {:else}
            <span class="text-xs">{time}</span>
          {/if}
        </span>
        {#if action === "unsnooze"}
          <button
            type="button"
            aria-label="Wake thread now"
            title="Wake thread now"
            onclick={stop(onUnsnooze)}
            class="pointer-events-none absolute inset-y-0 right-0 -mr-1 inline-flex cursor-pointer items-center gap-1 rounded-md bg-transparent px-1.5 text-xs text-muted-foreground opacity-0 transition-opacity group-hover/sidebar-row:pointer-events-auto group-hover/sidebar-row:opacity-100 hover:text-foreground focus-visible:pointer-events-auto focus-visible:opacity-100"
          >
            <AlarmClockOff class="mb-px size-3" />
          </button>
        {:else if action === "unsettle"}
          <button
            type="button"
            aria-label="Un-settle thread"
            title="Un-settle thread"
            onclick={stop(onUnsettle)}
            class="pointer-events-none absolute inset-y-0 right-0 -mr-1 inline-flex cursor-pointer items-center gap-1 rounded-md bg-transparent px-1.5 text-xs text-muted-foreground opacity-0 transition-opacity group-hover/sidebar-row:pointer-events-auto group-hover/sidebar-row:opacity-100 hover:text-foreground focus-visible:pointer-events-auto focus-visible:opacity-100"
          >
            <Undo2 class="mb-px size-3.5" />
          </button>
        {:else}
          <button
            type="button"
            aria-label="Settle thread"
            title="Settle thread"
            onclick={stop(onSettle)}
            class="pointer-events-none absolute inset-y-0 right-0 -mr-2 inline-flex cursor-pointer items-center gap-1 rounded-md bg-transparent px-2 text-xs text-muted-foreground opacity-0 transition-opacity group-hover/sidebar-row:pointer-events-auto group-hover/sidebar-row:opacity-100 hover:text-foreground focus-visible:pointer-events-auto focus-visible:opacity-100 {status ===
            'woke'
              ? 'group-hover/sidebar-row:static'
              : ''}"
          >
            <Check class="size-3" />
          </button>
        {/if}
        {@render jumpHint()}
      </span>
    </div>
  </li>
{:else}
  <li class="list-none py-0.5 [contain-intrinsic-size:auto_60px] [content-visibility:auto]">
    <div
      role="button"
      tabindex="0"
      data-testid="sidebar-row-card"
      data-thread-id={thread.id}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      title={tooltip}
      class={surface}
      onclick={onSelect}
      ondblclick={onDblclick}
      onkeydown={onKeydown}
      oncontextmenu={(event) => {
        event.preventDefault();
        onContextMenu(event.clientX, event.clientY);
      }}
    >
      <div class="relative z-10 h-auto px-(--sidebar-row-content-inset) py-(--sidebar-content-inset)">
        <div class="flex h-5 min-w-0 items-center gap-1.5">
          <ProjectIcon icon={thread.project.icon} name={thread.project.name} class="size-4 shrink-0" />
          {@render pinIndicator()}
          <span class="min-w-0 flex-1 truncate text-xs font-normal text-secondary-label">{thread.project.name}</span>
          <span
            class="group/sidebar-status-slot relative ml-auto flex h-5 min-w-8 shrink-0 items-stretch justify-end text-xs {jumpLabel
              ? '[&>:not([data-jump-hint])]:invisible [&>:not([data-jump-hint])]:absolute'
              : ''}"
          >
            <span
              class="flex items-center self-center justify-self-end text-secondary-label tabular-nums {status === 'woke'
                ? 'pointer-events-auto'
                : 'pointer-events-none group-hover/sidebar-row:absolute group-hover/sidebar-row:right-0 group-hover/sidebar-row:opacity-0 group-has-[:focus-visible]/sidebar-status-slot:absolute group-has-[:focus-visible]/sidebar-status-slot:right-0 group-has-[:focus-visible]/sidebar-status-slot:opacity-0'}"
            >
              {#if top && status === "woke"}
                <button
                  type="button"
                  aria-label="Dismiss Woke notification"
                  title="Dismiss Woke notification"
                  onclick={stop(onAcknowledgeWoke)}
                  class="inline-flex cursor-pointer items-center gap-1 rounded-sm font-medium outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset {top.class}"
                >
                  <AlarmClock aria-hidden="true" class="size-4 shrink-0" />
                  <span role="status">{top.label}</span>
                </button>
              {:else if top}
                <span class="inline-flex items-center gap-1 font-medium {top.class}">
                  <top.Icon class="size-4 shrink-0" />
                  {top.label}
                  {#if status === "working" && startedMs !== null}
                    <span aria-hidden="true" class="tabular-nums">{workingDuration(secondNow - startedMs)}</span>
                  {/if}
                </span>
              {:else}
                {time}
              {/if}
            </span>
            <span
              class="pointer-events-none absolute inset-y-0 right-0 flex items-stretch opacity-0 group-hover/sidebar-row:pointer-events-auto group-hover/sidebar-row:static group-hover/sidebar-row:opacity-100 has-[:focus-visible]:pointer-events-auto has-[:focus-visible]:static has-[:focus-visible]:opacity-100"
            >
              {#if !thread.waiting}
                <button
                  type="button"
                  aria-label="Snooze thread"
                  title="Snooze thread"
                  onclick={(event) => {
                    event.preventDefault();
                    event.stopPropagation();
                    const rect = event.currentTarget.getBoundingClientRect();
                    onSnoozeMenu(rect.right, rect.bottom + 4);
                  }}
                  ondblclick={(event) => event.stopPropagation()}
                  class="inline-flex h-full cursor-pointer items-center gap-0.5 rounded-md bg-transparent px-1.5 text-xs text-muted-foreground hover:text-foreground"
                >
                  <Clock class="size-3" />
                </button>
              {/if}
              <button
                type="button"
                aria-label="Settle thread"
                title="Settle thread"
                onclick={stop(onSettle)}
                class="-mr-1 inline-flex cursor-pointer items-center gap-1 rounded-md bg-transparent px-1.5 text-xs text-muted-foreground hover:text-foreground"
              >
                <Check class="size-3.5" />
                Settle
              </button>
            </span>
            {@render jumpHint()}
          </span>
        </div>
        <div class="mt-1 flex min-w-0 items-center gap-1.5">
          {@render title()}
        </div>
        {#if machine}
          <!-- T3 Code's meta line: the computer it's on, when there's more than one. -->
          <div class="mt-0.5 flex min-w-0 items-center gap-1.5 text-xs text-secondary-label" data-testid="sidebar-row-machine">
            <Monitor aria-hidden="true" class="size-3.5 shrink-0 text-muted-foreground/70" />
            <span class="min-w-0 flex-1 truncate">{machine}</span>
            {#if thread.project.offline}
              <span class="shrink-0">offline</span>
            {/if}
          </div>
        {/if}
      </div>
    </div>
  </li>
{/if}
