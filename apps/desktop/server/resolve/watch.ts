// Whether the user may be working in the project open in Resolve. Resolve doesn't say whether a project was
// changed, so this compares what each status check saw of it: the page, the timeline and its clips, the playhead.

/** A project nobody touched for this long is left without asking. */
export const QUIET_MS = 15 * 60_000;

export interface SeenProject {
  project: string;
  projectId: string | null;
  /** From status.py; undefined when Resolve didn't say. */
  activity: string | undefined;
}

export function createResolveWatch(now: () => number = Date.now) {
  let seen: { key: string; activity: string | undefined; since: number } | null = null;
  // Set when Linker-Edit itself changed the open project, so the next difference isn't the user's.
  let ownChange = false;

  return {
    /** After each status check; null when no project is open. */
    see(open: SeenProject | null) {
      if (!open) {
        seen = null;
      } else {
        const key = open.projectId ?? open.project;
        // Just opened, or watched for the first time: the user may have been in it until a moment ago.
        if (seen?.key !== key) seen = { key, activity: open.activity, since: now() };
        else if (seen.activity !== open.activity) seen = { key, activity: open.activity, since: ownChange ? seen.since : now() };
      }
      ownChange = false;
    },
    changedByUs() {
      ownChange = true;
    },
    /** Nothing in the open project moved for a while. False when that can't be told. */
    quiet: (): boolean => seen?.activity !== undefined && now() - seen.since >= QUIET_MS,
  };
}
