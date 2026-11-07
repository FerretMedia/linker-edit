import { execFile, spawn } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { Provider } from "../../shared/protocol.ts";
import type { ResolveMcpConfig } from "../config.ts";
import { needsApproval } from "../policy.ts";
import { RESOLVE_MCP_NAME } from "../rules.ts";
import { describeChanges } from "./diff.ts";
import {
  jsonRows,
  mediaEntries,
  parseTimelineFiles,
  storedTimeline,
  timelineFiles,
  timelineFolders,
  type RawSnapshot,
  type StoredTimeline,
} from "./model.ts";

/** Snapshots live under this folder of the workspace, one subfolder per Resolve project. */
export const HISTORY_ROOT = "resolve";

export interface AgentTurn {
  provider: Provider;
  prompt: string;
  /** Resolve write calls, e.g. `timeline.create_empty {"name":"AI - Cut"}`. */
  actions: string[];
  /** The agent's last reply. */
  summary: string;
}

export interface Saved {
  commit: string;
  changes: string[];
}

export interface History {
  /** Saves Resolve's open project. Without a turn the change is the user's own. Null when nothing changed. */
  record(turn?: AgentTurn): Promise<Saved | null>;
  /** The last few saves, for an agent starting a new chat. Empty when there is no history yet. */
  recentContext(): Promise<string>;
}

const SNAPSHOT_SCRIPT = path.join(import.meta.dirname, "snapshot.py");

export function readResolveSnapshot(mcp: ResolveMcpConfig): Promise<RawSnapshot> {
  return new Promise((resolve, reject) => {
    execFile(
      mcp.python,
      [SNAPSHOT_SCRIPT],
      { timeout: 120_000, maxBuffer: 256 * 1024 * 1024, windowsHide: true, env: { ...process.env, ...mcp.env } },
      (error, stdout) => {
        try {
          const result = JSON.parse(stdout.trim().split("\n").pop() ?? "") as RawSnapshot | { error: string };
          if ("error" in result) reject(new Error(result.error));
          else resolve(result);
        } catch {
          reject(error ?? new Error("The Resolve snapshot printed no result."));
        }
      },
    );
  });
}

function git(cwd: string, args: string[], input?: string): Promise<{ code: number; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn("git", args, { cwd, windowsHide: true });
    let stdout = "";
    let stderr = "";
    child.stdout.on("data", (chunk) => (stdout += chunk));
    child.stderr.on("data", (chunk) => (stderr += chunk));
    child.on("error", reject);
    child.on("close", (code) => resolve({ code: code ?? 1, stdout, stderr }));
    child.stdin.end(input ?? "");
  });
}

async function gitOk(cwd: string, args: string[], input?: string): Promise<string> {
  const result = await git(cwd, args, input);
  if (result.code !== 0) throw new Error(`git ${args[0]} failed: ${result.stderr.trim() || result.stdout.trim()}`);
  return result.stdout;
}

const AGENT_NAMES: Record<Provider, string> = { claude: "Claude", codex: "Codex" };
const RESOLVE_PREFIX = `mcp__${RESOLVE_MCP_NAME}__`;

/** `timeline.create_empty {"name":"AI - Cut"}` for a Resolve call that writes; null for reads and other tools. */
export function resolveWriteAction(tool: string | undefined, input: unknown): string | null {
  if (!tool?.startsWith(RESOLVE_PREFIX)) return null;
  const args = input && typeof input === "object" ? (input as Record<string, unknown>) : {};
  if (!needsApproval(tool, args)) return null;
  const action = typeof args.action === "string" ? args.action : "?";
  const params = args.params === undefined ? "" : ` ${JSON.stringify(args.params)}`;
  const line = `${tool.slice(RESOLVE_PREFIX.length)}.${action}${params}`;
  return line.length > 240 ? `${line.slice(0, 239)}…` : line;
}

/** Puts recent history ahead of the first message of a chat, so a fresh agent knows what changed. */
export function withHistoryContext(context: string, text: string): string {
  if (!context) return text;
  return `<edit_history>
Recent saves of the Resolve project, newest first, from the Git history in your working folder. Use git log and git show for details.
${context}
</edit_history>

${text}`;
}

function subjectOf(prompt: string): string {
  const line = prompt.replace(/\s+/g, " ").trim();
  return line.length > 72 ? `${line.slice(0, 71)}…` : line;
}

const bullets = (lines: string[]) => lines.map((line) => `- ${line}`).join("\n");

export function commitMessage(changes: string[], turn?: AgentTurn): string {
  const changeBlock = `Changes:\n${changes.length ? bullets(changes) : "- timeline details only"}`;
  if (!turn) return `Changes made in Resolve\n\n${changeBlock}\n`;
  const summary = turn.summary.length > 1500 ? `${turn.summary.slice(0, 1500)}…` : turn.summary;
  return [
    subjectOf(turn.prompt) || "Agent changes",
    `Prompt: ${turn.prompt}`,
    `Resolve actions:\n${turn.actions.length ? bullets(turn.actions) : "- none"}`,
    changeBlock,
    ...(summary ? [`Agent summary:\n${summary}`] : []),
  ].join("\n\n") + "\n";
}

function readStored(projectDir: string): StoredTimeline[] {
  if (!fs.existsSync(projectDir)) return [];
  return fs
    .readdirSync(projectDir, { withFileTypes: true })
    .filter((entry) => entry.isDirectory())
    .map((entry) => {
      const folder = path.join(projectDir, entry.name);
      return parseTimelineFiles((file) => {
        const full = path.join(folder, file);
        return fs.existsSync(full) ? fs.readFileSync(full, "utf8") : null;
      });
    })
    .filter((timeline) => timeline !== null);
}

function writeStored(projectDir: string, snapshot: RawSnapshot, timelines: StoredTimeline[]) {
  const folders = timelineFolders(timelines.map((timeline) => timeline.metadata.name));
  fs.mkdirSync(projectDir, { recursive: true });
  for (const entry of fs.readdirSync(projectDir, { withFileTypes: true })) {
    if (entry.isDirectory() && !folders.includes(entry.name)) {
      fs.rmSync(path.join(projectDir, entry.name), { recursive: true });
    }
  }
  timelines.forEach((timeline, index) => {
    const folder = path.join(projectDir, folders[index]!);
    fs.mkdirSync(folder, { recursive: true });
    for (const [file, contents] of Object.entries(timelineFiles(timeline))) {
      fs.writeFileSync(path.join(folder, file), contents);
    }
  });
  fs.writeFileSync(path.join(projectDir, "media.json"), jsonRows("media", mediaEntries(snapshot)));
}

export function createHistory(options: { dir: string; readSnapshot: () => Promise<RawSnapshot> }): History {
  const { dir } = options;
  let identity: Promise<string[]> | null = null;
  // Saves run one at a time, so a turn's "before" save always lands before its "after" save.
  let queue: Promise<unknown> = Promise.resolve();

  function prepare(): Promise<string[]> {
    identity ??= (async () => {
      fs.mkdirSync(dir, { recursive: true });
      if ((await git(dir, ["rev-parse", "--is-inside-work-tree"])).code !== 0) await gitOk(dir, ["init", "-q"]);
      const name = (await git(dir, ["config", "user.name"])).stdout.trim();
      if (name) return [];
      const user = os.userInfo().username || "Linker-Edit user";
      return ["-c", `user.name=${user}`, "-c", `user.email=${user}@linker-edit.local`];
    })();
    return identity;
  }

  async function save(turn?: AgentTurn): Promise<Saved | null> {
    const fallbackIdentity = await prepare();
    const snapshot = await options.readSnapshot();
    const projectDir = path.join(dir, HISTORY_ROOT, timelineFolders([snapshot.project])[0]!);
    const before = readStored(projectDir);
    const after = snapshot.timelines.map(storedTimeline);
    if (before.length === 0 && after.length === 0) return null;
    writeStored(projectDir, snapshot, after);

    const pathspec = path.relative(dir, projectDir).replaceAll("\\", "/");
    await gitOk(dir, ["add", "-A", "--", pathspec]);
    if ((await git(dir, ["diff", "--cached", "--quiet", "--", pathspec])).code === 0) return null;

    const changes = describeChanges(before, after);
    const author = turn ? [`--author=${AGENT_NAMES[turn.provider]} <${turn.provider}@linker-edit.local>`] : [];
    await gitOk(
      dir,
      [...fallbackIdentity, "commit", "-q", "--no-verify", ...author, "-F", "-", "--", pathspec],
      commitMessage(changes, turn),
    );
    const commit = (await gitOk(dir, ["rev-parse", "--short", "HEAD"])).trim();
    return { commit, changes };
  }

  async function recentContext(): Promise<string> {
    await prepare();
    const log = await git(dir, [
      "log",
      "-n",
      "5",
      "--date=format:%Y-%m-%d %H:%M",
      "--format=%x1e%h%x1f%ad%x1f%an%x1f%B",
      "--",
      HISTORY_ROOT,
    ]);
    if (log.code !== 0) return "";
    return log.stdout
      .split("\x1e")
      .filter((entry) => entry.trim())
      .map((entry) => {
        const [hash, date, author, body = ""] = entry.split("\x1f");
        const [subject] = body.split("\n");
        const changes = /\nChanges:\n((?:- .*\n?)*)/.exec(body)?.[1]?.trimEnd() ?? "";
        return `${hash} ${date} ${author}: ${subject}${changes ? `\n${changes.replace(/^/gm, "  ")}` : ""}`;
      })
      .join("\n");
  }

  return {
    record(turn) {
      const result = queue.then(() => save(turn));
      queue = result.catch(() => {});
      return result;
    },
    recentContext: () => {
      const result = queue.then(recentContext);
      queue = result.catch(() => {});
      return result;
    },
  };
}
