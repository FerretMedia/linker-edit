// Releases are named by their day, like Linker's: 2026.11.07 is the release of 7 November 2026 (UTC). That's the name
// on tags, GitHub releases, installers and in the app. A day has one release; releasing again that day replaces it.
//
// npm and the app updater need a version of three numbers without leading zeros that sorts in release order, so
// underneath each release is YYYY.MDD.HHMM, the UTC time it was made: 2026.1107.1430. A release that replaces the
// day's earlier one has a later time, so installed apps still update to it.

const VERSION = /^(\d{4})\.(\d{1,2})(\d{2})\.(\d+)$/;

/** The release name for a version: 2026.1107.1430 → 2026.11.07. Anything else, like a checkout's 0.0.0, comes back as
 * it is. */
export function releaseName(version: string): string {
  const match = VERSION.exec(version);
  if (!match) return version;
  const [, year, month, day] = match as unknown as [string, string, string, string];
  return `${year}.${month.padStart(2, "0")}.${day}`;
}

/** Where every release is, to download one by hand. */
export const RELEASES_URL = "https://github.com/FerretMedia/linker-edit/releases/latest";

/** Whether version a comes after version b; both are YYYY.MDD.HHMM. */
export function isNewer(a: string, b: string): boolean {
  const x = a.split(".").map(Number);
  const y = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    const diff = (x[i] ?? 0) - (y[i] ?? 0);
    if (diff !== 0) return diff > 0;
  }
  return false;
}
