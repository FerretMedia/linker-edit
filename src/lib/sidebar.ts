// The sidebar's thread and project logic, ported from T3 Code (MIT, see NOTICE):
// packages/client-runtime/src/state/threadSort.ts, threadSettled.ts, threadInbox.ts and
// apps/web/src/components/Sidebar.logic.ts.

import type { ProjectSummary, ThreadSummary } from "#shared/protocol.ts";

export type SidebarSection = "pinned" | "active" | "snoozed" | "settled";

/** A thread with its project, as the sidebar lists it. */
export interface SidebarThread extends ThreadSummary {
  project: ProjectSummary;
}

/** Threads a project folder shows before "Show more". */
export const FOLDER_PREVIEW_COUNT = 6;
/** Settled threads shown when the shelf opens, and how many more each "Show more" adds. As in T3 Code. */
export const SETTLED_INITIAL_COUNT = 10;
export const SETTLED_PAGE_COUNT = 25;

const time = (iso: string | null | undefined): number | null => {
  if (!iso) return null;
  const ms = Date.parse(iso);
  return Number.isNaN(ms) ? null : ms;
};

const byId = (left: { id: string }, right: { id: string }) => (left.id < right.id ? -1 : left.id > right.id ? 1 : 0);

/** Snoozed and still asleep. A thread waiting for an approval wakes early, as one asking for input does in T3 Code. */
export function isSnoozed(thread: ThreadSummary, nowMs: number): boolean {
  const wake = time(thread.snoozedUntil);
  return wake !== null && wake > nowMs && !thread.waiting;
}

/** When a snoozed thread woke: at its time, or early when it asked for an approval. */
export function wokeAt(thread: ThreadSummary, nowMs: number): string | null {
  const wake = time(thread.snoozedUntil);
  if (wake === null) return null;
  if (thread.waiting) return thread.snoozedAt ?? thread.snoozedUntil;
  return wake <= nowMs ? thread.snoozedUntil : null;
}

/** Woke since it was last looked at. */
export function isWoke(thread: ThreadSummary, nowMs: number): boolean {
  const woke = time(wokeAt(thread, nowMs));
  if (woke === null || thread.settledAt) return false;
  const read = time(thread.lastReadAt);
  return read === null || read < woke;
}

/** A turn ended after the user last looked. A thread never looked at isn't unread, as in T3 Code. */
export function isUnread(thread: ThreadSummary): boolean {
  if (thread.busy) return false;
  const ended = time(thread.turnEndedAt);
  const read = time(thread.lastReadAt);
  return ended !== null && read !== null && ended > read;
}

export function threadSection(thread: ThreadSummary, nowMs: number): SidebarSection {
  if (isSnoozed(thread, nowMs)) return "snoozed";
  if (thread.settledAt) return "settled";
  if (thread.pinnedAt) return "pinned";
  return "active";
}

export type ThreadStatus = "approval" | "working" | "ready";

export function threadStatus(thread: ThreadSummary): ThreadStatus {
  if (thread.waiting) return "approval";
  if (thread.busy) return "working";
  return "ready";
}

/** What the row says on its right, before the time: T3 Code's "top status". */
export type TopStatus = "working" | "approval" | "woke" | "done";

export function topStatus(thread: ThreadSummary, nowMs: number): TopStatus | null {
  const status = threadStatus(thread);
  if (status === "working") return "working";
  if (status === "approval") return "approval";
  if (isWoke(thread, nowMs)) return "woke";
  if (isUnread(thread)) return "done";
  return null;
}

/** Quiet rows: working, or done and seen. The open thread never recedes. */
export function shouldRecede(thread: ThreadSummary, nowMs: number, active: boolean): boolean {
  if (active) return false;
  const status = threadStatus(thread);
  if (status === "working") return true;
  return !isUnread(thread) && !isWoke(thread, nowMs);
}

/** The time a thread's activity shows, and active threads sort by: its creation or the user's last message. */
export function activityTime(thread: ThreadSummary): number {
  return Math.max(time(thread.createdAt) ?? 0, time(thread.lastSentAt ?? thread.updatedAt) ?? 0);
}

export function activityTimestamp(thread: ThreadSummary): string {
  return new Date(activityTime(thread) || Date.parse(thread.updatedAt)).toISOString();
}

/** Newest send first. */
export function sortActive<T extends ThreadSummary>(threads: readonly T[]): T[] {
  return [...threads].sort((left, right) => activityTime(right) - activityTime(left) || byId(left, right));
}

/** Pinned threads have no manual order here (T3 Code's drag order isn't ported): newest first. */
export function sortPinned<T extends ThreadSummary>(threads: readonly T[]): T[] {
  return [...threads].sort((left, right) => (time(right.createdAt) ?? 0) - (time(left.createdAt) ?? 0) || byId(left, right));
}

/** Soonest to wake first. */
export function sortSnoozed<T extends ThreadSummary>(threads: readonly T[]): T[] {
  return [...threads].sort((left, right) => (time(left.snoozedUntil) ?? 0) - (time(right.snoozedUntil) ?? 0) || byId(left, right));
}

export function settledTimestamp(thread: ThreadSummary): string {
  return thread.settledAt ?? activityTimestamp(thread);
}

/** Most recently settled first. */
export function sortSettled<T extends ThreadSummary>(threads: readonly T[]): T[] {
  return [...threads].sort((left, right) => Date.parse(settledTimestamp(right)) - Date.parse(settledTimestamp(left)) || byId(left, right));
}

/** A project's last activity: its newest thread's. */
export function projectActivity(project: ProjectSummary): number {
  return project.threads.filter((thread) => !thread.archived).reduce((latest, thread) => Math.max(latest, activityTime(thread)), 0);
}

/** Pinned projects first, then by last activity; archived ones last. */
export function sortProjects(projects: readonly ProjectSummary[]): ProjectSummary[] {
  const rank = (project: ProjectSummary) => (project.archived ? 2 : project.pinned ? 0 : 1);
  return [...projects].sort(
    (left, right) => rank(left) - rank(right) || projectActivity(right) - projectActivity(left) || left.name.localeCompare(right.name),
  );
}

export interface SidebarFolder {
  project: ProjectSummary;
  threads: SidebarThread[];
}

export interface SidebarLists {
  pinned: SidebarThread[];
  folders: SidebarFolder[];
  snoozed: SidebarThread[];
  settled: SidebarThread[];
  /** Everything listed, in order, for moving to the next thread after a settle. */
  order: SidebarThread[];
}

/**
 * The sidebar's sections. A project gets a folder when it has active threads, or when it's pinned (not archived)
 * or open in Resolve; an archived project keeps one only for its active threads, below the rest.
 */
export function partitionSidebar(projects: readonly ProjectSummary[], nowMs: number): SidebarLists {
  const pinned: SidebarThread[] = [];
  const snoozed: SidebarThread[] = [];
  const settled: SidebarThread[] = [];
  const folders: SidebarFolder[] = [];
  for (const project of sortProjects(projects)) {
    const active: SidebarThread[] = [];
    for (const thread of project.threads) {
      if (thread.archived) continue;
      const listed = { ...thread, project };
      const section = threadSection(thread, nowMs);
      (section === "pinned" ? pinned : section === "snoozed" ? snoozed : section === "settled" ? settled : active).push(listed);
    }
    if (active.length > 0 || (!project.archived && (project.pinned || project.open))) {
      folders.push({ project, threads: sortActive(active) });
    }
  }
  const lists = { pinned: sortPinned(pinned), folders, snoozed: sortSnoozed(snoozed), settled: sortSettled(settled) };
  return { ...lists, order: [...lists.pinned, ...folders.flatMap((folder) => folder.threads)] };
}

/** The next thread to open after settling or snoozing the open one: the next listed one, wrapping around. */
export function nextThreadAfter(order: readonly SidebarThread[], threadId: string): SidebarThread | null {
  const index = order.findIndex((thread) => thread.id === threadId);
  if (index === -1) return null;
  return [...order.slice(index + 1), ...order.slice(0, index)][0] ?? null;
}

const fold = (text: string) => text.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();

/**
 * Threads whose title (or project name) matches every word of the query: title starts first, then word starts,
 * then anywhere; ties by activity. Settled and archived threads are found too.
 */
export function searchThreads(projects: readonly ProjectSummary[], query: string): SidebarThread[] {
  const words = fold(query).split(/\s+/).filter(Boolean);
  if (words.length === 0) return [];
  const scored: { thread: SidebarThread; score: number }[] = [];
  for (const project of projects) {
    for (const thread of project.threads) {
      const title = fold(thread.title);
      const subject = `${title} ${fold(project.name)}`;
      if (!words.every((word) => subject.includes(word))) continue;
      const first = words[0]!;
      const score = title.startsWith(first) ? 0 : new RegExp(`(^|[^\\p{L}\\p{N}])${escapeRegExp(first)}`, "u").test(title) ? 1 : title.includes(first) ? 2 : 3;
      scored.push({ thread: { ...thread, project }, score });
    }
  }
  return scored
    .sort((left, right) => left.score - right.score || activityTime(right.thread) - activityTime(left.thread) || byId(left.thread, right.thread))
    .map(({ thread }) => thread);
}

const escapeRegExp = (text: string) => text.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");

/** "now", "5m", "3h", "2d": T3 Code's compact sidebar time. */
export function relativeTime(iso: string, nowMs: number): string {
  const ms = Date.parse(iso);
  if (Number.isNaN(ms)) return "";
  const seconds = Math.floor(Math.max(0, nowMs - ms) / 1000);
  if (seconds < 60) return "now";
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

/** How long until a snoozed thread wakes: "45m", "3h", "2d". */
export function snoozeWakeLabel(snoozedUntil: string, nowMs: number): string {
  const wakeMs = Date.parse(snoozedUntil);
  if (Number.isNaN(wakeMs) || wakeMs <= nowMs) return "now";
  const minutes = Math.max(1, Math.floor(wakeMs / 60_000) - Math.floor(nowMs / 60_000));
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 24 * 60) return `${Math.ceil(minutes / 60)}h`;
  return `${Math.ceil(minutes / (24 * 60))}d`;
}

type SnoozePresetSetting = { kind: "after"; minutes: number } | { kind: "at"; day: "today" | "tomorrow" | "monday"; time: string };

/** T3 Code's default snooze choices. */
const SNOOZE_PRESETS: SnoozePresetSetting[] = [
  { kind: "after", minutes: 60 },
  { kind: "after", minutes: 180 },
  { kind: "at", day: "today", time: "18:00" },
  { kind: "at", day: "tomorrow", time: "09:00" },
  { kind: "at", day: "monday", time: "09:00" },
];

export interface SnoozePreset {
  id: string;
  label: string;
  /** The wake time, shown after the label. */
  whenLabel: string;
  snoozedUntil: string;
}

const pluralize = (count: number, unit: string) => `${count} ${unit}${count === 1 ? "" : "s"}`;
const timeOfDay = (date: Date) => date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });

function resolvePreset(setting: SnoozePresetSetting, now: Date): SnoozePreset | null {
  let wake: Date;
  let daysAhead = 0;
  let label: string;
  if (setting.kind === "after") {
    wake = new Date(now.getTime() + setting.minutes * 60_000);
    label = `In ${setting.minutes % 60 === 0 ? pluralize(setting.minutes / 60, "hour") : pluralize(setting.minutes, "minute")}`;
  } else {
    daysAhead = setting.day === "today" ? 0 : setting.day === "tomorrow" ? 1 : (1 - now.getDay() + 7) % 7 || 7;
    wake = new Date(now);
    wake.setDate(wake.getDate() + daysAhead);
    wake.setHours(Number(setting.time.slice(0, 2)), Number(setting.time.slice(3)), 0, 0);
    if (wake.getTime() <= now.getTime()) return null;
    const hour = Number(setting.time.slice(0, 2));
    label =
      daysAhead === 0
        ? hour < 12
          ? "This morning"
          : hour < 17
            ? "This afternoon"
            : "This evening"
        : daysAhead === 1
          ? "Tomorrow"
          : "Next week";
  }
  const namesDay = setting.kind === "at" && label !== "Next week";
  return {
    id: setting.kind === "after" ? `after:${setting.minutes}` : `at:${setting.day}:${setting.time}`,
    label,
    whenLabel:
      namesDay || wake.toDateString() === now.toDateString()
        ? timeOfDay(wake)
        : `${wake.toLocaleDateString(undefined, { weekday: "short" })} ${timeOfDay(wake)}`,
    snoozedUntil: wake.toISOString(),
  };
}

/** The snooze menu: soonest first, one row per wake minute. */
export function snoozePresets(now: Date): SnoozePreset[] {
  const minute = (preset: SnoozePreset) => Math.floor(Date.parse(preset.snoozedUntil) / 60_000);
  const seen = new Set<number>();
  return SNOOZE_PRESETS.flatMap((setting) => resolvePreset(setting, now) ?? [])
    .sort((a, b) => minute(a) - minute(b))
    .filter((preset) => !seen.has(minute(preset)) && seen.add(minute(preset)));
}

/** The next time any snoozed thread wakes, so the sidebar re-sorts then without a ticking clock. */
export function nextWake(projects: readonly ProjectSummary[], nowMs: number): number | null {
  let next: number | null = null;
  for (const project of projects) {
    for (const thread of project.threads) {
      const wake = time(thread.snoozedUntil);
      if (wake !== null && wake > nowMs && (next === null || wake < next)) next = wake;
    }
  }
  return next;
}
