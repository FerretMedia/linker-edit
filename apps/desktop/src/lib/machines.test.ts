import { describe, expect, it } from "vitest";
import type { Machine, ProjectSummary } from "#shared/protocol.ts";
import { activeMachine, cacheMachine, diffMachines, forgetMachines, parseMachines, readCache, readSelection, saveSelection, THIS_MACHINE } from "./machines.ts";

function store() {
  const items = new Map<string, string>();
  return {
    items,
    getItem: (key: string) => items.get(key) ?? null,
    setItem: (key: string, value: string) => void items.set(key, value),
    removeItem: (key: string) => void items.delete(key),
  };
}

const machine = (id: string, fields: Partial<Machine> = {}): Machine => ({ id, name: id, version: "2026.11.20", via: "linker", online: true, base: `/machines/${id}`, ...fields });

describe("the machine list", () => {
  it("puts this computer first, and is this computer alone for a server older than the list", () => {
    const rig = machine("rig");
    const here = { ...THIS_MACHINE, name: "Studio-PC" };
    expect(parseMachines({ machines: [rig, here] })).toEqual([here, rig]);
    expect(parseMachines({ machines: [rig] })).toEqual([THIS_MACHINE, rig]);
    expect(parseMachines({ error: "Not found" })).toBeNull();
    expect(parseMachines(null)).toBeNull();
  });

  it("only takes machines reached through this computer's own server", () => {
    const list = parseMachines({ machines: [machine("a", { base: "https://evil.example" }), machine("b", { base: "/machines/b/../../x" }), { id: "c" }, machine("d")] });
    expect(list?.map((entry) => entry.id)).toEqual(["this", "d"]);
  });

  it("connects the new machines, updates the known ones and leaves the gone ones", () => {
    const { add, update, remove } = diffMachines(["this", "rig", "old"], [THIS_MACHINE, machine("rig"), machine("laptop")]);
    expect(add.map((entry) => entry.id)).toEqual(["laptop"]);
    expect(update.map((entry) => entry.id)).toEqual(["this", "rig"]);
    expect(remove).toEqual(["old"]);
  });
});

describe("the open thread", () => {
  it("is shown on its machine once that machine has something to show, and on the first one that has until then", () => {
    const selection = { machineId: "rig", threadId: "t1" };
    expect(activeMachine(selection, [{ id: "this", ready: true }, { id: "rig", ready: true }])).toBe("rig");
    expect(activeMachine(selection, [{ id: "this", ready: true }, { id: "rig", ready: false }])).toBe("this");
    expect(activeMachine(null, [{ id: "this", ready: false }, { id: "rig", ready: true }])).toBe("rig");
    expect(activeMachine(selection, [{ id: "this", ready: false }])).toBeNull();
  });

  it("is remembered with its machine, as a thread id is only unique there", () => {
    const local = store();
    expect(readSelection(local)).toBeNull();
    saveSelection(local, { machineId: "rig", threadId: "t1" });
    expect(readSelection(local)).toEqual({ machineId: "rig", threadId: "t1" });
    local.setItem("linker-edit:selection", "not json");
    expect(readSelection(local)).toBeNull();
  });
});

describe("offline machines", () => {
  it("keep the projects they had, until signing out", () => {
    const local = store();
    const projects = [{ id: "p", name: "Promo", threads: [] }] as unknown as ProjectSummary[];
    cacheMachine(local, "rig", { name: "Edit-Rig", projects });
    cacheMachine(local, "laptop", { name: "Laptop", projects: [] });
    expect(readCache(local)).toEqual({ rig: { name: "Edit-Rig", projects }, laptop: { name: "Laptop", projects: [] } });

    saveSelection(local, { machineId: "rig", threadId: null });
    forgetMachines(local);
    expect(readCache(local)).toEqual({});
    expect(readSelection(local)).toBeNull();
  });
});
