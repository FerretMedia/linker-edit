// Prints the next release version, YYYY.MDD.N: 2026.1108.0 is the first release on 8 November 2026
// (UTC), 2026.1108.1 the second. npm needs three numbers without leading zeros, which this keeps sortable.
//
//   node scripts/release-version.ts $(git tag --list 'v*')

export function nextVersion(date: Date, tags: string[]): string {
  const day = `${date.getUTCFullYear()}.${(date.getUTCMonth() + 1) * 100 + date.getUTCDate()}`;
  const taken = tags
    .filter((tag) => tag.startsWith(`v${day}.`))
    .map((tag) => tag.slice(`v${day}.`.length))
    .filter((n) => /^\d+$/.test(n))
    .map(Number);
  return `${day}.${taken.length === 0 ? 0 : Math.max(...taken) + 1}`;
}

if (import.meta.main) console.log(nextVersion(new Date(), process.argv.slice(2)));
