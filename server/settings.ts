// The user's choices that outlive a restart, in ~/LinkerEdit/settings.json. Kept apart from
// linker-edit.config.json, which the user writes by hand and Linker-Edit only reads.
import * as fs from "node:fs";
import * as path from "node:path";
import type { ApprovalMode } from "../shared/protocol.ts";

export interface Settings {
  approvalMode?: ApprovalMode;
}

export interface SettingsStore {
  load(): Settings;
  save(changes: Settings): void;
}

const APPROVAL_MODES = new Set<unknown>(["ask", "auto"]);

/** Reads only what it knows; a missing or broken file is no settings, never an error that stops the app. */
export function fileSettings(file: string, onError: (error: unknown) => void = console.error): SettingsStore {
  function load(): Settings {
    let raw: unknown;
    try {
      raw = JSON.parse(fs.readFileSync(file, "utf8"));
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") onError(error);
      return {};
    }
    const mode = raw && typeof raw === "object" ? (raw as Record<string, unknown>).approvalMode : undefined;
    return APPROVAL_MODES.has(mode) ? { approvalMode: mode as ApprovalMode } : {};
  }
  return {
    load,
    save(changes) {
      try {
        fs.mkdirSync(path.dirname(file), { recursive: true });
        const temp = `${file}.tmp`;
        fs.writeFileSync(temp, `${JSON.stringify({ ...load(), ...changes }, null, 2)}\n`);
        fs.renameSync(temp, file);
      } catch (error) {
        onError(error);
      }
    },
  };
}
