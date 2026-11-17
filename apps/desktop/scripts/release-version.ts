// Prints the release's version and name for the release workflow's output, as `version=…` and `name=…` lines.
// A release at 14:30 UTC on 8 November 2026 is version 2026.1108.1430, named 2026.11.08. See shared/release-name.ts.
//
//   node scripts/release-version.ts
import { releaseName } from "#shared/release-name.ts";

export function releaseVersion(date: Date): string {
  const day = (date.getUTCMonth() + 1) * 100 + date.getUTCDate();
  const time = date.getUTCHours() * 100 + date.getUTCMinutes();
  return `${date.getUTCFullYear()}.${day}.${time}`;
}

if (import.meta.main) {
  const version = releaseVersion(new Date());
  console.log(`version=${version}\nname=${releaseName(version)}`);
}
