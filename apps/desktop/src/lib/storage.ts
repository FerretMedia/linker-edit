import type { ComputerStorage, StorageCard } from "#shared/protocol.ts";
import type { Host } from "./host.ts";
import type { MachineConnection } from "./machines.svelte.ts";

const UNITS = ["B", "KB", "MB", "GB", "TB", "PB"];

/** Bytes as Windows Explorer counts them, a GB being 1024³ bytes, to three figures: "4.53 GB", "45.3 GB", "453 GB". */
export function formatBytes(bytes: number): string {
  let value = bytes;
  let unit = 0;
  while (value >= 1000 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit === 0 || value >= 100 ? 0 : value >= 10 ? 1 : 2;
  return `${value.toFixed(digits)} ${UNITS[unit]}`;
}

/**
 * A computer's storage card. In the desktop app this computer's server asks your other computers and keeps what they
 * said last. This computer, and each computer on your own web address, is asked straight, so an offline one there
 * has nothing to show.
 */
export async function loadStorageCard(entry: MachineConnection, home: Host): Promise<StorageCard> {
  const kept = !entry.host.web && entry.machine.via !== "this";
  if (!kept && entry.status === "offline") return { state: "none", problem: `${entry.name} is offline.` };
  try {
    const res = kept
      ? await home.api(`/api/machines/storage?id=${encodeURIComponent(entry.id)}`, { cache: "no-store" })
      : await entry.host.api("/api/storage", { cache: "no-store" });
    const body: unknown = await res.json().catch(() => null);
    const error = (body as { error?: unknown } | null)?.error;
    if (!res.ok) return { state: "none", problem: typeof error === "string" ? error : `${entry.name} answered ${res.status}.` };
    // An older Linker-Edit answers with its page instead.
    if (body === null) return { state: "none", problem: `Update Linker-Edit on ${entry.name} to see its storage.` };
    return kept ? (body as StorageCard) : { state: "fresh", report: body as ComputerStorage };
  } catch (error) {
    return { state: "none", problem: error instanceof Error ? error.message : String(error) };
  }
}
