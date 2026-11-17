import { versionStamp } from "./blobs.ts";
import type { DrpFile } from "./read.ts";

// The project formats seen in sample files, for messages. Resolve opens older formats (upgrading them) but
// refuses newer ones.
const FORMATS: Record<number, string> = { 14: "Resolve 19.1", 17: "Resolve 21" };

export type VersionCheck = { ok: true } | { ok: false; message: string };

/** Whether a Resolve that reads project format `targetPrjVer` (its DbPrjVer) can open this file. */
export function checkVersion(file: DrpFile, targetPrjVer: number): VersionCheck {
  const label = file.project.name ? `"${file.project.name}"` : "This file";
  const stamps = [...file.xml].map(([entry, text]) => ({ entry, stamp: versionStamp(text) }));
  const unstamped = stamps.find(({ stamp }) => !stamp);
  if (unstamped || !stamps.length) {
    return { ok: false, message: `${label} has no version stamp${unstamped ? ` in ${unstamped.entry}` : ""}, so it can't be checked.` };
  }
  const formats = [...new Set(stamps.map(({ stamp }) => stamp!.dbPrjVer))].sort((a, b) => a - b);
  if (formats.length > 1) {
    return { ok: false, message: `${label} mixes project formats ${formats.join(" and ")}, so it wasn't saved by one Resolve.` };
  }
  const { dbAppVer, dbPrjVer } = stamps[0]!.stamp!;
  if (dbPrjVer <= targetPrjVer) return { ok: true };
  const app = dbAppVer.split(".").slice(0, 3).join(".");
  const target = FORMATS[targetPrjVer] ? ` (${FORMATS[targetPrjVer]})` : "";
  return {
    ok: false,
    message:
      `${label} was saved by DaVinci Resolve ${app} in project format ${dbPrjVer}, but the Resolve opening it ` +
      `reads format ${targetPrjVer}${target}. Resolve can't open projects from a newer version: update it to ${app} or later.`,
  };
}
