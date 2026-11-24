import { describe, expect, it } from "vitest";
import { createResolveWatch, QUIET_MS } from "./watch.ts";

function watching() {
  let time = 0;
  const watch = createResolveWatch(() => time);
  const see = (activity: string | undefined, projectId = "promo-id") => watch.see({ project: "Promo", projectId, activity });
  return { watch, see, wait: (ms: number) => (time += ms) };
}

describe("createResolveWatch", () => {
  it("is quiet only once nothing moved for a while, counted from when it was first seen", () => {
    const { watch, see, wait } = watching();
    expect(watch.quiet()).toBe(false);
    see("a");
    wait(QUIET_MS - 1);
    see("a");
    expect(watch.quiet()).toBe(false);
    wait(1);
    expect(watch.quiet()).toBe(true);
    see("b");
    expect(watch.quiet()).toBe(false);
  });

  it("starts over for another project, and after Resolve had none open", () => {
    const { watch, see, wait } = watching();
    see("a");
    wait(QUIET_MS);
    see("a", "doc-id");
    expect(watch.quiet()).toBe(false);
    wait(QUIET_MS);
    watch.see(null);
    see("a", "doc-id");
    expect(watch.quiet()).toBe(false);
  });

  it("doesn't take Linker-Edit's own changes for the user's, only until the next check", () => {
    const { watch, see, wait } = watching();
    see("a");
    wait(QUIET_MS);
    watch.changedByUs();
    see("b");
    expect(watch.quiet()).toBe(true);
    see("c");
    expect(watch.quiet()).toBe(false);
  });

  it("is never quiet when Resolve doesn't say what is in the project", () => {
    const { watch, see, wait } = watching();
    see(undefined);
    wait(QUIET_MS);
    expect(watch.quiet()).toBe(false);
  });
});
