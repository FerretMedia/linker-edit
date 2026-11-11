// Releases are named by their day, like Linker's: 2026.11.07 is the first release on 7 November 2026 (UTC), and
// 2026.11.07.2 the second. That's the name on tags, GitHub releases, installers and in the app.
//
// npm and the app updater need a version of three numbers without leading zeros that sorts in release order, so
// underneath each release is YYYY.MDD.N: 2026.1107.0 and 2026.1107.1. The two map one to one.

const VERSION = /^(\d{4})\.(\d{1,2})(\d{2})\.(\d+)$/;
const NAME = /^(\d{4})\.(\d{2})\.(\d{2})(?:\.(\d+))?$/;

/** The release name for a version: 2026.1107.0 → 2026.11.07, 2026.1107.1 → 2026.11.07.2. Anything else, like a
 * checkout's 0.0.0, comes back as it is. */
export function releaseName(version: string): string {
  const match = VERSION.exec(version);
  if (!match) return version;
  const [, year, month, day, n] = match as unknown as [string, string, string, string, string];
  const name = `${year}.${month.padStart(2, "0")}.${day}`;
  return n === "0" ? name : `${name}.${Number(n) + 1}`;
}

/** The version under a release name, or null when it isn't one: 2026.11.07.2 → 2026.1107.1. */
export function versionOfName(name: string): string | null {
  const match = NAME.exec(name);
  if (!match) return null;
  const [, year, month, day, n] = match as unknown as [string, string, string, string, string | undefined];
  if (n !== undefined && Number(n) < 2) return null;
  return `${year}.${Number(month) * 100 + Number(day)}.${n === undefined ? 0 : Number(n) - 1}`;
}
