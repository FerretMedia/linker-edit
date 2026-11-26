import type { LinkerOrg } from "../../shared/protocol.ts";
import type { LinkerService } from "../linker.ts";
import { exampleName, NEW_COLLECTION, RULES_FILE, type CollectionStore, type SavedCollection } from "./store.ts";

/** A CollectionStore in memory, for app tests. */
export function memoryCollectionStore(
  initial: SavedCollection[] = [],
): CollectionStore & { collections: Map<string, SavedCollection>; exampleNotes: Map<string, string> } {
  const collections = new Map(initial.map((collection) => [collection.id, structuredClone(collection)]));
  // "<collection id>/<example name>" → its notes.
  const exampleNotes = new Map<string, string>();
  const exampleFile = (id: string, name: string) => `/collections/${id}/examples/${name}.md`;
  let next = 0;
  return {
    collections,
    exampleNotes,
    load: () => [...collections.values()].map((collection) => structuredClone(collection)),
    create(fields) {
      next += 1;
      const collection: SavedCollection = { ...structuredClone(NEW_COLLECTION), ...structuredClone(fields), id: `collection-${next}` };
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
    examples: (id) =>
      [...exampleNotes.keys()]
        .filter((key) => key.startsWith(`${id}/`))
        .sort()
        .map((key) => ({ name: key.slice(id.length + 1), file: exampleFile(id, key.slice(id.length + 1)) })),
    saveExample(id, name, text) {
      if (!collections.has(id)) return null;
      exampleNotes.set(`${id}/${exampleName(name)}`, text);
      return exampleFile(id, exampleName(name));
    },
    removeExample(id, name) {
      exampleNotes.delete(`${id}/${exampleName(name)}`);
    },
  };
}

/** A Linker account signed in with these orgs, for app tests: each org is a brand. */
export function linkerWithOrgs(orgs: LinkerOrg[]): LinkerService {
  return {
    status: () => ({ state: "signed_in", name: "Jane", email: null, picture: null, plan: null, appUrl: "https://app.example" }),
    onStatus: () => {},
    orgs: () => orgs,
    listOrgs: async () => {},
  } as Partial<LinkerService> as LinkerService;
}
