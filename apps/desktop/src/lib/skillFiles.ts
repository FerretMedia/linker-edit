import type { SkillImportResult, SkippedFile } from "#shared/protocol.ts";
import { formatBytes, hiddenPart, SKILL_IMPORT_LIMITS as LIMITS } from "#shared/skillImport.ts";

/** A file dropped or picked, by its path in what was dropped, such as "my-skills/color/SKILL.md". */
export interface PickedFile {
  path: string;
  file: File;
}

const ARCHIVE = /\.(zip|skill)$/i;

/** Everything dropped, folders read through. Call it in the drop handler itself: the items are gone after it. */
export function droppedFiles(transfer: DataTransfer): Promise<PickedFile[]> {
  const entries = [...transfer.items].flatMap((item) => item.webkitGetAsEntry() ?? []);
  const found: PickedFile[] = [];
  const walk = async (entry: FileSystemEntry): Promise<void> => {
    const path = entry.fullPath.replace(/^\/+/, "");
    if (entry.isFile) {
      found.push({ path, file: await new Promise<File>((resolve, reject) => (entry as FileSystemFileEntry).file(resolve, reject)) });
      return;
    }
    // A hidden folder such as .git can hold thousands of files; it is reported as one and never read.
    if (hiddenPart(path)) {
      found.push({ path, file: new File([], entry.name) });
      return;
    }
    const reader = (entry as FileSystemDirectoryEntry).createReader();
    // Each call gives up to 100 entries, then an empty list at the end.
    for (;;) {
      const batch = await new Promise<FileSystemEntry[]>((resolve, reject) => reader.readEntries(resolve, reject));
      if (!batch.length) return;
      for (const child of batch) await walk(child);
    }
  };
  return Promise.all(entries.map(walk)).then(() => found);
}

/** Files from a file or folder picker; a folder's keep their path inside it. */
export const pickedFiles = (files: FileList): PickedFile[] => [...files].map((file) => ({ path: file.webkitRelativePath || file.name, file }));

function base64(file: File): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result).slice(String(reader.result).indexOf(",") + 1));
    reader.onerror = () => reject(reader.error ?? new Error(`Couldn't read ${file.name}.`));
    reader.readAsDataURL(file);
  });
}

/**
 * The files to send for an import, read as base64, and those left out before reading: hidden ones and files too big
 * for a skill. The server leaves out more, with its own reasons.
 */
export async function readForImport(picked: PickedFile[]): Promise<{ files: { path: string; base64: string }[]; skipped: SkippedFile[] } | { error: string }> {
  const skipped: SkippedFile[] = [];
  const sending: PickedFile[] = [];
  for (const entry of picked) {
    const hidden = hiddenPart(entry.path);
    if (hidden) {
      if (!skipped.some((skip) => skip.path === hidden)) skipped.push({ path: hidden, reason: "It's hidden." });
    } else if (entry.file.size > LIMITS.fileBytes && !ARCHIVE.test(entry.path)) {
      skipped.push({ path: entry.path, reason: `It's over ${formatBytes(LIMITS.fileBytes)}.` });
    } else sending.push(entry);
  }
  const size = sending.reduce((sum, entry) => sum + entry.file.size, 0);
  if (size > LIMITS.dropBytes) return { error: `That's ${formatBytes(size)}. Drop up to ${formatBytes(LIMITS.dropBytes)} at a time.` };
  try {
    return { files: await Promise.all(sending.map(async (entry) => ({ path: entry.path, base64: await base64(entry.file) }))), skipped };
  } catch (error) {
    return { error: error instanceof Error ? error.message : String(error) };
  }
}

/** "Imported 3 skills, replaced 1, skipped 2." */
export function importSummary(result: SkillImportResult): string {
  const { imported, replaced, skipped } = result;
  const parts = [`Imported ${imported.length || "no"} skill${imported.length === 1 ? "" : "s"}`];
  if (replaced.length) parts.push(`replaced ${replaced.length}`);
  if (skipped.length) parts.push(`skipped ${skipped.length}`);
  return `${parts.join(", ")}.`;
}
