// Prints the next release's version and name for the release workflow's output, as `version=…` and `name=…` lines.
// The first release on 8 November 2026 (UTC) is version 2026.1108.0, named 2026.11.08; the second is 2026.1108.1,
// named 2026.11.08.2. See shared/release-name.ts for why there are two.
//
//   node scripts/release-version.ts $(git tag --list 'v*')
import { releaseName, versionOfName } from "#shared/release-name.ts";

/** The version a release tag stands for. Tags are v<name>; releases before the names were tagged v<version>. */
function versionOfTag(tag: string): string | null {
  const bare = tag.replace(/^v/, "");
  return /^\d{4}\.\d{3,4}\.\d+$/.test(bare) ? bare : versionOfName(bare);
}

export function nextVersion(date: Date, tags: string[]): string {
  const day = `${date.getUTCFullYear()}.${(date.getUTCMonth() + 1) * 100 + date.getUTCDate()}`;
  const taken = tags
    .map(versionOfTag)
    .filter((version) => version?.startsWith(`${day}.`))
    .map((version) => Number(version!.slice(`${day}.`.length)));
  return `${day}.${taken.length === 0 ? 0 : Math.max(...taken) + 1}`;
}

if (import.meta.main) {
  const version = nextVersion(new Date(), process.argv.slice(2));
  console.log(`version=${version}\nname=${releaseName(version)}`);
}
