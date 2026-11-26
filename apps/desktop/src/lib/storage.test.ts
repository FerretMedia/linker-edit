import { expect, it } from "vitest";
import { formatBytes } from "./storage.ts";

it("counts bytes as Explorer does, 1 GB being 1024³ bytes, to three figures", () => {
  const sizes = [0, 999, 1536, 45.3 * 1024 ** 3, 453.7 * 1024 ** 3, 4.5 * 1024 ** 4];
  expect(sizes.map(formatBytes)).toEqual(["0 B", "999 B", "1.50 KB", "45.3 GB", "454 GB", "4.50 TB"]);
});
