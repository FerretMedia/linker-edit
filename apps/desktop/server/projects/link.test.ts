import { describe, expect, it } from "vitest";
import type { LocalLink, ResolveLink } from "../../shared/protocol.ts";
import { isLinkedProject, linkKind, type OpenProject } from "./link.ts";

const disk = { type: "Disk", name: "Local Database" };

describe("linkKind", () => {
  it.each([
    [null, "local"],
    [disk, "local"],
    [{ type: "PostgreSQL", name: "Team", host: "10.0.0.5" }, "network"],
    [{ type: "Cloud", name: "Blackmagic Cloud" }, "cloud"],
  ] as const)("%j → %s", (database, kind) => {
    expect(linkKind(database)).toBe(kind);
  });
});

describe("isLinkedProject", () => {
  const link = (fields: Partial<ResolveLink> = {}): ResolveLink => ({ kind: "local", name: "Doc", projectId: null, ...fields });
  const open = (fields: Partial<OpenProject> = {}): OpenProject => ({ name: "Doc", id: "doc-id", database: disk, ...fields });

  it.each<[string, ResolveLink, LocalLink, OpenProject, boolean]>([
    ["the same id, even after a rename in Resolve", link({ projectId: "doc-id" }), {}, open({ name: "Doc v2" }), true],
    ["another id with the same name", link({ projectId: "other" }), {}, open(), false],
    ["the name, before the id is known", link(), {}, open(), true],
    ["the name, when Resolve reports no id", link({ projectId: "doc-id" }), {}, open({ id: null }), true],
    ["another name", link(), {}, open({ name: "Promo" }), false],
    ["the name in another kind of library", link({ kind: "cloud" }), {}, open(), false],
    ["the name in this machine's database", link(), { database: disk }, open(), true],
    ["the name in another local database", link(), { database: { type: "Disk", name: "Archive" } }, open(), false],
  ])("matches %s", (_name, linkValue, local, openValue, expected) => {
    expect(isLinkedProject(linkValue, local, openValue)).toBe(expected);
  });
});
