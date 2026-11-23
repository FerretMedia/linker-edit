// The sidebar's thread and project logic, ported from T3 Code (MIT, see NOTICE):
// packages/client-runtime/src/state/threadSort.ts, threadSettled.ts, threadInbox.ts and
// apps/web/src/components/Sidebar.tsx and Sidebar.logic.ts.

import type { ProjectSummary, ThreadSummary } from "#shared/protocol.ts";
import { scopedKey } from "./machines.ts";

/** T3 Code's shelves, top to bottom. Working exists only in the inbox, not in project folders. */
export type SidebarSection = "pinned" | "active" | "working" | "snoozed" | "settled";

/** A project with its machine, as the sidebar lists every machine's together. */
export interface SidebarProject extends ProjectSummary {
  machineId: string;
  /** Unique across machines, where its own id may not be. */
  key: string;
  /** Its machine isn't connected: shown as it was last, and nothing goes to it. */
  offline: boolean;
}

/** A thread with its project, as the sidebar lists it. */
export interface SidebarThread extends ThreadSummary {
  project: SidebarProject;
  /** Unique across machines, where its own id may not be. */
  key: string;
}

/**
 * Every machine's projects in one list, like T3 Code's environments. An offline machine's are what it had last; a
 * turn that was running then isn't shown as still running, as nobody can tell.
 */
export function machineProjects(machines: readonly { id: string; projects: readonly ProjectSummary[]; offline: boolean }[]): SidebarProject[] {
  return machines.flatMap((machine) =>
    machine.projects.map((project) => ({
      ...project,
      threads: machine.offline ? project.threads.map((thread) => ({ ...thread, busy: false, waiting: false })) : project.threads,
      machineId: machine.id,
      key: scopedKey(machine.id, project.id),
      offline: machine.offline,
    })),
  );
}

const listed = (thread: ThreadSummary, project: SidebarProject): SidebarThread => ({ ...thread, project, key: scopedKey(project.machineId, thread.id) });

/** Every machine's threads but the archived ones, for the working tracker to look at. */
export const listedThreads = (projects: readonly SidebarProject[]): SidebarThread[] =>
  projects.flatMap((project) => project.threads.filter((thread) => !thread.archived).map((thread) => listed(thread, project)));

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
const byKey = (left: { key: string }, right: { key: string }) => (left.key < right.key ? -1 : left.key > right.key ? 1 : 0);

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

/** Where a thread goes before the Working shelf takes the running ones out of Active. */
export function threadSection(thread: ThreadSummary, nowMs: number): "pinned" | "active" | "snoozed" | "settled" {
  if (isSnoozed(thread, nowMs)) return "snoozed";
  if (thread.settledAt) return "settled";
  if (thread.pinnedAt) return "pinned";
  return "active";
}

/** A turn is running and nothing waits on the user: T3 Code's isThreadWorking. One asking for an approval needs you, so it's Active. */
export const isWorking = (thread: ThreadSummary) => thread.busy && !thread.waiting;

/** How long a thread that stopped working stays on the Working shelf, in case its next step starts. As in T3 Code. */
export const WORKING_EXIT_GRACE_MS = 2_000;
/** A turn that ended this long ago gets no grace: it ended while this page was closed. */
const STALE_EXIT_MS = 60_000;

/** What the working tracker saw: which threads are on the Working shelf, and when each came back from it. */
export interface WorkingView {
  isWorking: (thread: SidebarThread) => boolean;
  returnedAt: (thread: SidebarThread) => number | undefined;
}

/**
 * T3 Code's inbox return tracker (threadInbox.ts, createInboxReturnTracker): a thread whose turn ends stays on the
 * Working shelf for WORKING_EXIT_GRACE_MS, so it doesn't jump at once, then goes back to the top of Active. Only
 * where it's listed waits; an approval, or the user moving it, counts at once. Keep one for the page's life.
 */
export function createWorkingTracker() {
  const states = new Map<string, { working: boolean; deadline: number | null; placement: string | null }>();
  const returns = new Map<string, number>();
  return {
    observe(threads: readonly SidebarThread[], now: number): WorkingView {
      const present = new Set<string>();
      for (const thread of threads) {
        present.add(thread.key);
        const previous = states.get(thread.key);
        const actual = isWorking(thread);
        const placement = actual || previous?.working ? JSON.stringify([thread.archived, thread.pinnedAt, thread.settledAt, thread.snoozedUntil]) : null;
        const moved = previous !== undefined && previous.placement !== placement;
        const excluded = thread.archived || !!thread.pinnedAt || !!thread.settledAt || isSnoozed(thread, now);
        const ended = time(thread.turnEndedAt);
        const endedLongAgo = ended !== null && ended <= now - STALE_EXIT_MS;
        const deadline =
          !actual && previous?.working && !thread.waiting && !moved && !excluded && !endedLongAgo
            ? (previous.deadline ?? now + WORKING_EXIT_GRACE_MS)
            : null;
        const working = actual || (deadline !== null && deadline > now);
        if (previous?.working && !working) returns.set(thread.key, now);
        states.set(thread.key, { working, deadline: working ? deadline : null, placement });
      }
      for (const key of states.keys()) {
        if (!present.has(key)) {
          states.delete(key);
          returns.delete(key);
        }
      }
      const working = new Map([...states].map(([key, state]) => [key, state.working]));
      const returned = new Map(returns);
      return { isWorking: (thread) => working.get(thread.key) ?? isWorking(thread), returnedAt: (thread) => returned.get(thread.key) };
    },
    /** When the next grace ends, to look again then; null when none is running. */
    nextDeadline(): number | null {
      let next: number | null = null;
      for (const state of states.values()) if (state.deadline !== null && (next === null || state.deadline < next)) next = state.deadline;
      return next;
    },
  };
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
export function sortProjects<P extends ProjectSummary>(projects: readonly P[]): P[] {
  const rank = (project: ProjectSummary) => (project.archived ? 2 : project.pinned ? 0 : 1);
  return [...projects].sort(
    (left, right) => rank(left) - rank(right) || projectActivity(right) - projectActivity(left) || left.name.localeCompare(right.name),
  );
}

export interface SidebarFolder {
  project: SidebarProject;
  threads: SidebarThread[];
}

/** The newest of several times, as T3 Code's sorts take them. */
const newest = (...values: (string | null | undefined)[]) => Math.max(0, ...values.map((value) => time(value) ?? 0));

/**
 * The inbox's Active order (T3 Code's sortInboxThreadsByReturn): newest return first, a return being its creation,
 * the user's last message, its last turn ending, coming out of Settled, or leaving the Working shelf.
 */
export function sortInbox(threads: readonly SidebarThread[], returnedAt: (thread: SidebarThread) => number | undefined = () => undefined): SidebarThread[] {
  const at = (thread: SidebarThread) =>
    Math.max(newest(thread.createdAt, thread.unsettledAt, thread.lastSentAt, thread.turnEndedAt), returnedAt(thread) ?? 0);
  return [...threads].sort((left, right) => at(right) - at(left) || byKey(left, right));
}

/** The Working shelf's order (T3 Code's sortWorkingThreadsBySend): newest send first. */
export function sortWorking(threads: readonly SidebarThread[]): SidebarThread[] {
  const at = (thread: SidebarThread) => newest(thread.createdAt, thread.lastSentAt);
  return [...threads].sort((left, right) => at(right) - at(left) || byKey(left, right));
}

export interface SidebarLists {
  pinned: SidebarThread[];
  /** The inbox's Active threads, from every listed project; empty with project folders. */
  active: SidebarThread[];
  /** Turns running; empty with project folders, which keep them in place. */
  working: SidebarThread[];
  /** With project folders only: the active threads by project. */
  folders: SidebarFolder[] | null;
  snoozed: SidebarThread[];
  settled: SidebarThread[];
  /** Pinned, Active and Working, in order: for the jump shortcuts and the next thread after a settle. */
  order: SidebarThread[];
}

export interface SidebarOptions {
  /** Project keys to list, T3 Code's project scope; null or empty lists every project. */
  scope?: ReadonlySet<string> | null;
  /** T3 Code's boring sidebar: active threads in a folder per project, and no Working shelf. */
  folders?: boolean;
  /** The working tracker's view; without it a thread is on the Working shelf exactly while its turn runs. */
  working?: WorkingView;
}

/** The projects a scope lists: all of them, or the ones in it. A scope naming no listed project lists all. */
export function scopedProjects(projects: readonly SidebarProject[], scope: ReadonlySet<string> | null | undefined): SidebarProject[] {
  const inScope = scope ? projects.filter((project) => scope.has(project.key)) : [];
  return inScope.length > 0 ? inScope : [...projects];
}

/**
 * The sidebar's shelves, as T3 Code has them: Pinned, Active, Working, Snoozed and Settled, every machine's threads
 * mixed. With folders, Active is a folder per project instead: a project gets one when it has active threads, or
 * when it's pinned (not archived), open in Resolve or picked in the scope; an archived project keeps one only for
 * its active threads, below the rest.
 */
export function partitionSidebar(projects: readonly SidebarProject[], nowMs: number, options: SidebarOptions = {}): SidebarLists {
  const listedProjects = scopedProjects(projects, options.scope);
  const scoped = listedProjects.length < projects.length;
  const working = options.folders ? null : (options.working ?? { isWorking, returnedAt: () => undefined });
  const lists = { pinned: [] as SidebarThread[], active: [] as SidebarThread[], working: [] as SidebarThread[], snoozed: [] as SidebarThread[], settled: [] as SidebarThread[] };
  const folders: SidebarFolder[] = [];
  for (const project of sortProjects(listedProjects)) {
    const active: SidebarThread[] = [];
    for (const summary of project.threads) {
      if (summary.archived) continue;
      const thread = listed(summary, project);
      const section = threadSection(thread, nowMs);
      if (section !== "active") lists[section].push(thread);
      else if (working?.isWorking(thread)) lists.working.push(thread);
      else active.push(thread);
    }
    lists.active.push(...active);
    if (options.folders && (active.length > 0 || (!project.archived && (project.pinned || project.open || scoped)))) {
      folders.push({ project, threads: sortActive(active) });
    }
  }
  const pinned = sortPinned(lists.pinned);
  const active = options.folders ? [] : sortInbox(lists.active, working?.returnedAt);
  const workingShelf = sortWorking(lists.working);
  return {
    pinned,
    active,
    working: workingShelf,
    folders: options.folders ? folders : null,
    snoozed: sortSnoozed(lists.snoozed),
    settled: sortSettled(lists.settled),
    order: [...pinned, ...(options.folders ? folders.flatMap((folder) => folder.threads) : active), ...workingShelf],
  };
}

/** The next thread to open after settling or snoozing the open one: the next listed one, wrapping around. */
export function nextThreadAfter(order: readonly SidebarThread[], key: string): SidebarThread | null {
  const index = order.findIndex((thread) => thread.key === key);
  if (index === -1) return null;
  return [...order.slice(index + 1), ...order.slice(0, index)][0] ?? null;
}

const fold = (text: string) => text.normalize("NFKD").replace(/\p{M}/gu, "").toLowerCase();

/**
 * Threads whose title (or project name) matches every word of the query: title starts first, then word starts,
 * then anywhere; ties by activity. Settled and archived threads are found too.
 */
export function searchThreads(projects: readonly SidebarProject[], query: string): SidebarThread[] {
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
      scored.push({ thread: listed(thread, project), score });
    }
  }
  return scored
    .sort(
      (left, right) =>
        left.score - right.score || activityTime(right.thread) - activityTime(left.thread) || byKey(left.thread, right.thread),
    )
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

/** How long a turn has run, after "Working": "12s", "4m", "1h 5m". T3 Code's formatWorkingDurationLabel. */
export function workingDuration(elapsedMs: number): string {
  const seconds = Number.isFinite(elapsedMs) ? Math.max(0, Math.floor(elapsedMs / 1000)) : 0;
  if (seconds < 60) return `${seconds}s`;
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) return `${minutes}m`;
  return `${Math.floor(minutes / 60)}h ${minutes % 60}m`;
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
