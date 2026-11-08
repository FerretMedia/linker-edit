import * as fs from "node:fs";
import * as path from "node:path";
import type { ChatItem, ContextUsage, Provider } from "../shared/protocol.ts";

/** The open chat, kept so a restart continues it. */
export interface SavedChat {
  provider: Provider;
  /** The provider's conversation id; null before the first turn. */
  resumeId: string | null;
  items: ChatItem[];
  context: ContextUsage | null;
}

export interface ChatStore {
  load(): SavedChat | null;
  save(chat: SavedChat): void;
}

// Bump when SavedChat changes shape; a file with another version is ignored rather than misread.
const VERSION = 1;

/** Keeps the chat in <workspace>/.linker-edit/chat.json, a folder Git ignores. */
export function fileChatStore(workspaceDir: string, onError: (error: unknown) => void = console.error): ChatStore {
  const dir = path.join(workspaceDir, ".linker-edit");
  const file = path.join(dir, "chat.json");
  return {
    load() {
      try {
        const saved = JSON.parse(fs.readFileSync(file, "utf8")) as { version?: unknown; chat?: SavedChat };
        return saved.version === VERSION && saved.chat ? saved.chat : null;
      } catch (error) {
        if ((error as NodeJS.ErrnoException).code !== "ENOENT") onError(error);
        return null;
      }
    },
    save(chat) {
      try {
        fs.mkdirSync(dir, { recursive: true });
        fs.writeFileSync(path.join(dir, ".gitignore"), "*\n");
        // Written whole and renamed, so a crash mid-write can't leave half a file.
        const temp = `${file}.tmp`;
        fs.writeFileSync(temp, JSON.stringify({ version: VERSION, chat }));
        fs.renameSync(temp, file);
      } catch (error) {
        onError(error);
      }
    },
  };
}
