import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import { describe, expect, it } from "vitest";
import { fileChatStore, type SavedChat } from "./chatStore.ts";

const chat: SavedChat = {
  provider: "codex",
  resumeId: "thread-1",
  items: [{ id: "1", kind: "user", text: "Cut the silences" }],
  context: null,
};

function workspace() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "linker-edit-chat-"));
}

describe("fileChatStore", () => {
  it("keeps the chat in a folder Git ignores", () => {
    const dir = workspace();
    fileChatStore(dir).save(chat);
    expect(fileChatStore(dir).load()).toEqual(chat);
    expect(fs.readFileSync(path.join(dir, ".linker-edit", ".gitignore"), "utf8")).toBe("*\n");
  });

  it.each([
    ["no file yet", null, 0],
    ["a file from another version", JSON.stringify({ version: 99, chat }), 0],
    ["a damaged file", '{"version": 1, "chat": {', 1],
  ])("starts empty with %s", (_name, contents, errors) => {
    const dir = workspace();
    if (contents !== null) {
      fs.mkdirSync(path.join(dir, ".linker-edit"));
      fs.writeFileSync(path.join(dir, ".linker-edit", "chat.json"), contents);
    }
    const reported: unknown[] = [];
    expect(fileChatStore(dir, (error) => reported.push(error)).load()).toBeNull();
    expect(reported).toHaveLength(errors);
  });
});
