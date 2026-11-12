import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { fileSettings } from "./settings.ts";

const tempFile = () => path.join(fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-settings-")), "settings.json");

describe("settings", () => {
  it("keeps the approval mode across restarts", () => {
    const file = tempFile();
    expect(fileSettings(file).load()).toEqual({});
    fileSettings(file).save({ approvalMode: "ask" });
    expect(fileSettings(file).load()).toEqual({ approvalMode: "ask" });
  });

  it("ignores a broken file and values it doesn't know", () => {
    const file = tempFile();
    const errors: unknown[] = [];
    fs.writeFileSync(file, "{ not json");
    expect(fileSettings(file, (error) => errors.push(error)).load()).toEqual({});
    expect(errors).toHaveLength(1);
    fs.writeFileSync(file, JSON.stringify({ approvalMode: "yes" }));
    expect(fileSettings(file).load()).toEqual({});
  });
});
