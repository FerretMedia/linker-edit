import { PARENT_KINDS, type CollectionKind, type CollectionSummary } from "#shared/protocol.ts";

/** The Brands page's address. Like the settings page's, a reload keeps it and Back closes it. */
export const BRANDS_HASH = "#brands";

/** A collection and what it is in, root first, itself last. */
export function lineage(collections: CollectionSummary[], collection: CollectionSummary): CollectionSummary[] {
  const byId = new Map(collections.map((entry) => [entry.id, entry]));
  const chain: CollectionSummary[] = [];
  for (let at: CollectionSummary | undefined = collection; at; at = at.parentId ? byId.get(at.parentId) : undefined) chain.unshift(at);
  return chain;
}

/** It takes new things: its brand is one the person is in, or it is in no brand at all. */
export function isOpen(collections: CollectionSummary[], collection: CollectionSummary): boolean {
  const root = lineage(collections, collection)[0]!;
  return root.kind !== "brand" || root.org?.member === true;
}

/** "Ferret Media / Shorts / Weekly". */
export const pathName = (collections: CollectionSummary[], collection: CollectionSummary) =>
  lineage(collections, collection)
    .map((entry) => entry.name)
    .join(" / ");

/** A collection and everything in it, depth first, each with how deep it is. */
export function subtree(collections: CollectionSummary[], root: CollectionSummary, depth = 0): { collection: CollectionSummary; depth: number }[] {
  return [
    { collection: root, depth },
    ...collections.filter((entry) => entry.parentId === root.id).flatMap((child) => subtree(collections, child, depth + 1)),
  ];
}

/** The tops of the trees: brands first, then sub-brands and series from before brands. */
export const roots = (collections: CollectionSummary[]) => collections.filter((entry) => entry.parentId === null);

/** Every place a project can go, as paths in tree order; a place in a brand the person left can't be picked. */
export const placeOptions = (collections: CollectionSummary[]) =>
  roots(collections)
    .flatMap((root) => subtree(collections, root))
    .map(({ collection }) => ({
      id: collection.id,
      kind: collection.kind,
      label: pathName(collections, collection),
      disabled: !isOpen(collections, collection),
    }));

/** Where a collection of this kind can go, as paths. An older computer still says "workspace": a sub-brand now. */
export const parentOptions = (collections: CollectionSummary[], kind: CollectionKind) =>
  placeOptions(collections).filter((option) => (PARENT_KINDS[kind] ?? PARENT_KINDS.subbrand).includes(option.kind));
