// Gathers the desktop builds the release workflow downloaded into one folder for the GitHub release.
//
//   node scripts/release-assets.ts <downloaded artifacts> <release folder>
//
// Each build writes an update manifest (latest.yml, latest-mac.yml, latest-linux.yml) listing only its own files.
// The two Mac builds both write latest-mac.yml, so their file lists are merged: the app picks its architecture's.
import * as fs from "node:fs";
import * as path from "node:path";

/** One manifest with every manifest's `files` entries. They're electron-builder's YAML, all for the same version. */
export function mergeManifests(manifests: string[]): string {
  const [first, ...rest] = manifests.map((text) => text.replace(/\r\n/g, "\n").trimEnd().split("\n"));
  if (!first) throw new Error("No manifests to merge.");
  const filesOf = (lines: string[]) => {
    const start = lines.indexOf("files:");
    if (start === -1) throw new Error("A manifest has no files list.");
    let end = start + 1;
    while (lines[end]?.startsWith(" ")) end++;
    return { start, end };
  };
  const { end } = filesOf(first);
  const added = rest.flatMap((lines) => {
    const { start, end } = filesOf(lines);
    return lines.slice(start + 1, end);
  });
  return [...first.slice(0, end), ...added, ...first.slice(end)].join("\n") + "\n";
}

const ASSET = /\.(exe|dmg|zip|AppImage|blockmap)$/;

if (import.meta.main) {
  const [from, to] = process.argv.slice(2);
  if (!from || !to) {
    console.error("Usage: release-assets <downloaded artifacts> <release folder>");
    process.exit(1);
  }
  fs.mkdirSync(to, { recursive: true });
  const manifests = new Map<string, string[]>();
  for (const file of fs.globSync("**/*", { cwd: from })) {
    const source = path.join(from, file);
    if (!fs.statSync(source).isFile()) continue;
    const name = path.basename(file);
    if (/^latest(-\w+)?\.yml$/.test(name)) {
      manifests.set(name, [...(manifests.get(name) ?? []), fs.readFileSync(source, "utf8")]);
    } else if (ASSET.test(name)) {
      if (fs.existsSync(path.join(to, name))) throw new Error(`Two builds made ${name}.`);
      fs.copyFileSync(source, path.join(to, name));
    }
  }
  for (const [name, texts] of manifests) fs.writeFileSync(path.join(to, name), mergeManifests(texts));
  for (const name of fs.readdirSync(to).sort()) console.log(name);
}
