// The folder walks of storage.ts, on a thread of their own so a big project never holds up the server.
import { parentPort } from "node:worker_threads";
import { walkFolder, type FolderSize, type Listings } from "./walk.ts";

export interface WalkRequest {
  id: number;
  roots: string[];
}

export interface WalkAnswer {
  id: number;
  /** In the order of roots; null for a folder that isn't there. */
  sizes: (FolderSize | null)[];
}

/** Each root's listings from its last walk; a root no longer asked about is forgotten. */
let kept = new Map<string, Listings>();

parentPort!.on("message", ({ id, roots }: WalkRequest) => {
  const next = new Map<string, Listings>();
  const sizes = roots.map((root) => {
    const { size, listings } = walkFolder(root, kept.get(root) ?? new Map(), Date.now());
    next.set(root, listings);
    return size;
  });
  kept = next;
  parentPort!.postMessage({ id, sizes } satisfies WalkAnswer);
});
