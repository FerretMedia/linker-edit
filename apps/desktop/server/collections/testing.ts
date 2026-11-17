import { RULES_FILE, type CollectionStore, type SavedCollection } from "./store.ts";

/** A CollectionStore in memory, for app tests. */
export function memoryCollectionStore(initial: SavedCollection[] = []): CollectionStore & { collections: Map<string, SavedCollection> } {
  const collections = new Map(initial.map((collection) => [collection.id, structuredClone(collection)]));
  let next = 0;
  return {
    collections,
    load: () => [...collections.values()].map((collection) => structuredClone(collection)),
    create(fields) {
      next += 1;
      const collection: SavedCollection = { ...structuredClone(fields), id: `collection-${next}`, library: [], musicOnlyFromHere: false };
      collections.set(collection.id, structuredClone(collection));
      return collection;
    },
    save(collection) {
      if (collections.has(collection.id)) collections.set(collection.id, structuredClone(collection));
    },
    remove(id) {
      collections.delete(id);
    },
    rulesFile: (id) => (collections.has(id) ? `/collections/${id}/${RULES_FILE}` : null),
  };
}
