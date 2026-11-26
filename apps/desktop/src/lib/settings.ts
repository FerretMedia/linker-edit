import type { ResolveStatus } from "#shared/protocol.ts";

export const SETTINGS_SECTIONS = [
  { id: "general", label: "General" },
  { id: "resolve", label: "Resolve" },
  { id: "transcription", label: "Transcription" },
  { id: "remote", label: "Remote access" },
  { id: "account", label: "Accounts" },
  { id: "about", label: "About & updates" },
] as const;

export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]["id"];

/** The settings page lives in the address's hash, so a reload or the back button keeps it: "#settings/remote". */
export function sectionFromHash(hash: string): SettingsSection | null {
  const match = /^#settings(?:\/([\w-]+))?$/.exec(hash);
  if (!match) return null;
  return SETTINGS_SECTIONS.find((section) => section.id === match[1])?.id ?? "general";
}

export const settingsHash = (section: SettingsSection) => `#settings/${section}`;

/** Resolve's state in one line, as the sidebar and the settings page show it. */
export function resolveStatusLine(resolve: ResolveStatus): { tone: string; title: string; detail: string } {
  switch (resolve.state) {
    case "checking":
      return { tone: "bg-warning", title: "Checking Resolve…", detail: "" };
    case "connected":
      return {
        tone: "bg-success",
        title: `${resolve.product} ${resolve.version.split(".").slice(0, 3).join(".")}`,
        detail: resolve.project ? `${resolve.project}${resolve.timeline ? ` · ${resolve.timeline}` : ""}` : "No project open",
      };
    case "not_running":
      return { tone: "bg-muted-foreground", title: "Resolve is not running", detail: "Agents start it when they need it. Or start it yourself, then refresh." };
    case "blocked":
      return { tone: "bg-warning", title: "Resolve needs attention", detail: resolve.hint };
    case "unreachable":
      return { tone: "bg-destructive", title: "Resolve not reachable", detail: resolve.hint };
    case "setup_needed":
      return { tone: "bg-destructive", title: "Setup needed", detail: resolve.hint };
  }
}
