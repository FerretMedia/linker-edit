import type { Computers, DesktopUpdate } from "#shared/protocol.ts";

/**
 * What the desktop window's preload (desktop/preload.cjs) offers pages; undefined in a browser. The update
 * methods are optional: an older app window may show the page of a newer Linker-Edit on another computer.
 */
export interface DesktopBridge {
  computers(): Promise<Computers & { home: string }>;
  open(url: string): Promise<void>;
  update?(): Promise<DesktopUpdate | null>;
  /** Returns a function that stops listening. */
  onUpdate?(listener: (update: DesktopUpdate) => void): () => void;
  installUpdate?(): Promise<void>;
  /** Whether pickFolder works for this page: only on this computer's own server. */
  canPickFolder?(): Promise<boolean>;
  /** The system's folder dialog; null when cancelled. */
  pickFolder?(defaultPath?: string): Promise<string | null>;
}

export const desktop = (globalThis as { linkerEditDesktop?: DesktopBridge }).linkerEditDesktop;
