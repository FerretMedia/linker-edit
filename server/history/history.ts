import { spawn } from "node:child_process";
import * as fs from "node:fs";
import * as os from "node:os";
import * as path from "node:path";
import type { HistoryEntry, PlayableTimeline, Provider } from "../../shared/protocol.ts";
import { isResolveWrite } from "../policy.ts";
import { PROJECT_FILE } from "../projects/store.ts";
import type { ResolveConnection } from "../resolve/connection.ts";
import { describeChanges, timelineStrips } from "./diff.ts";
import { mediaType, playableTimeline } from "./playable.ts";
import {
  jsonRows,
  mediaEntries,
  parseTimelineFiles,
  storedTimeline,
  timelineFiles,
  timelineFolders,
  TIMELINE_FILES,
  type MediaEntry,
  type RawSnapshot,
  type StoredTimeline,
} from "./model.ts";

/** Snapshots live under this folder of the workspace, one subfolder per Resolve project. */
export const HISTORY_ROOT = "resolve";

export interface AgentTurn {
  provider: Provider;
  prompt: string;
  /** Resolve write calls, e.g. `create_timeline {"name":"Cut"}`. */
  actions: string[];
  /** The agent's last reply. */
  summary: string;
  /** The Linker-Edit thread the turn ran in. */
  thread?: string;
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
  /** The newest saves, for the history panel. */
  log(limit?: number): Promise<HistoryEntry[]>;
  /** A timeline the browser can play, as Resolve has it now (commit null) or as it was in a save. */
  timeline(id: string | null, commit: string | null): Promise<PlayableTimeline | null>;
  /** A media file the browser can show or play; null for other formats and unknown ids. */
  mediaFile(id: string): { path: string; type: string } | null;
  /** Commits the project's linker-edit.json when it changed, so the link travels with the history. */
  commitProjectFile(message: string): Promise<void>;
}

/**
 * Says why Resolve's open project isn't the one this history belongs to, or null when it is. A save
 * refuses to write another project's timelines into this history.
 */
export type ProjectCheck = (open: { name: string; id: string | null }) => string | null;

/** Resolve's open project with every timeline, read in as many script runs as it takes. */
export async function readResolveSnapshot(connection: ResolveConnection): Promise<RawSnapshot> {
  type Page = RawSnapshot & { remaining: string[] };
  // Scripts stop after 60 seconds, so each run stops after 40 and says which timelines are left.
  const first = await connection.runScript<Page>("snapshot", { ids: null, budget: 40 }, 60);
  const timelines = [...first.timelines];
  let remaining = first.remaining;
  while (remaining.length) {
    const page = await connection.runScript<Page>("snapshot", { ids: remaining, budget: 40 }, 60);
    if (page.project_id !== first.project_id) throw new Error("Resolve switched projects while Linker-Edit was saving it.");
    if (page.timelines.length === 0) break;
    timelines.push(...page.timelines);
    remaining = page.remaining;
  }
  const { remaining: _, ...snapshot } = first;
  return { ...snapshot, timelines };
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
    // git can exit before it reads stdin. Its exit code reports any failure, so a broken pipe is no error.
    child.stdin.on("error", () => {});
    child.stdin.end(input ?? "");
  });
}

async function gitOk(cwd: string, args: string[], input?: string): Promise<string> {
  const result = await git(cwd, args, input);
  if (result.code !== 0) throw new Error(`git ${args[0]} failed: ${result.stderr.trim() || result.stdout.trim()}`);
  return result.stdout;
}

const AGENT_NAMES: Record<Provider, string> = { claude: "Claude", codex: "Codex" };

/** `create_timeline {"name":"Cut"}` for a Resolve call that writes; null for reads and other tools. */
export function resolveWriteAction(tool: string | undefined, input: unknown): string | null {
  if (!tool || !isResolveWrite(tool)) return null;
  const name = tool.split("__").at(-1)!;
  const args = input && typeof input === "object" ? JSON.stringify(input) : "";
  const line = args && args !== "{}" ? `${name} ${args}` : name;
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
    ...(turn.thread ? [`Thread: ${turn.thread}`] : []),
  ].join("\n\n") + "\n";
}

/** A bulleted section of a commit message written by commitMessage, without its "- none" placeholder. */
function section(body: string, title: string): string[] {
  const lines = new RegExp(`(?:^|\\n)${title}:\\n((?:- .*(?:\\n|$))*)`).exec(body)?.[1] ?? "";
  return lines
    .split("\n")
    .filter((line) => line.startsWith("- ") && line !== "- none")
    .map((line) => line.slice(2));
}

function parseEntry([commit = "", date = "", author = "", email = "", body = ""]: string[]): HistoryEntry {
  const agent = (Object.keys(AGENT_NAMES) as Provider[]).find((provider) => email === `${provider}@linker-edit.local`) ?? null;
  const prompt = /^Prompt: ([\s\S]*?)\n\nResolve actions:/m.exec(body)?.[1] ?? null;
  const thread = /\n\nThread: (\S+)\s*$/.exec(body)?.[1] ?? null;
  return {
    commit,
    date,
    author,
    agent,
    subject: body.split("\n")[0] ?? "",
    prompt,
    actions: section(body, "Resolve actions"),
    changes: section(body, "Changes"),
    thread,
    timelines: [],
  };
}

/** Contents of each `<rev>:<path>`, or null where it doesn't exist (a new timeline has no `<commit>^` side). */
function catFiles(cwd: string, names: string[]): Promise<(string | null)[]> {
  if (names.length === 0) return Promise.resolve([]);
  return new Promise((resolve, reject) => {
    const child = spawn("git", ["cat-file", "--batch"], { cwd, windowsHide: true });
    const chunks: Buffer[] = [];
    child.stdout.on("data", (chunk: Buffer) => chunks.push(chunk));
    child.on("error", reject);
    child.on("close", () => {
      const out = Buffer.concat(chunks);
      let at = 0;
      resolve(
        names.map(() => {
          const lineEnd = out.indexOf(10, at);
          if (lineEnd === -1) return null;
          const header = out.subarray(at, lineEnd).toString().split(" ");
          at = lineEnd + 1;
          if (header.length !== 3 || header[1] !== "blob" || !/^[0-9a-f]+$/.test(header[0]!)) return null;
          const size = Number(header[2]);
          const text = out.subarray(at, at + size).toString("utf8");
          at += size + 1;
          return text;
        }),
      );
    });
    child.stdin.end(`${names.join("\n")}\n`);
  });
}

/** Media pool item id → file path, from the media.json of every project in the history. */
function readMedia(dir: string): Map<string, MediaEntry> {
  const media = new Map<string, MediaEntry>();
  const root = path.join(dir, HISTORY_ROOT);
  if (!fs.existsSync(root)) return media;
  for (const project of fs.readdirSync(root, { withFileTypes: true })) {
    const file = path.join(root, project.name, "media.json");
    if (!project.isDirectory() || !fs.existsSync(file)) continue;
    try {
      for (const entry of (JSON.parse(fs.readFileSync(file, "utf8")) as { media: MediaEntry[] }).media) {
        media.set(entry.id, entry);
      }
    } catch {
      // A hand-edited or half-merged media.json only costs thumbnails.
    }
  }
  return media;
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

/** Says which Resolve project a history folder holds, so the folder can follow a rename in Resolve. */
const PROJECT_INFO = "project.json";

function readProjectId(folder: string): string | null {
  try {
    const info = JSON.parse(fs.readFileSync(path.join(folder, PROJECT_INFO), "utf8")) as { id?: unknown };
    return typeof info.id === "string" ? info.id : null;
  } catch {
    return null;
  }
}

/**
 * The folder a snapshot of this Resolve project goes in, named after the project. When Resolve renamed the
 * project, its folder is renamed too, so its history goes on instead of starting over in a new folder.
 */
function projectFolder(dir: string, snapshot: RawSnapshot): { folder: string; renamedFrom: string | null } {
  const root = path.join(dir, HISTORY_ROOT);
  const folder = path.join(root, timelineFolders([snapshot.project])[0]!);
  const id = snapshot.project_id;
  if (!id || readProjectId(folder) === id || !fs.existsSync(root)) return { folder, renamedFrom: null };
  for (const entry of fs.readdirSync(root, { withFileTypes: true })) {
    const old = path.join(root, entry.name);
    if (!entry.isDirectory() || old === folder || readProjectId(old) !== id) continue;
    // Another folder already has the new name: keep saving where this project's history is.
    if (fs.existsSync(folder)) return { folder: old, renamedFrom: null };
    fs.renameSync(old, folder);
    return { folder, renamedFrom: old };
  }
  return { folder, renamedFrom: null };
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
  if (snapshot.project_id) {
    fs.writeFileSync(path.join(projectDir, PROJECT_INFO), `${JSON.stringify({ id: snapshot.project_id, name: snapshot.project }, null, 2)}\n`);
  }
}

export function createHistory(options: {
  dir: string;
  readSnapshot: () => Promise<RawSnapshot>;
  checkProject?: ProjectCheck;
}): History {
  const { dir } = options;
  let identity: Promise<string[]> | null = null;
  // Media seen in live snapshots, so the quick preview finds media no save has recorded yet.
  const snapshotMedia = new Map<string, MediaEntry>();
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
    const refusal = options.checkProject?.({ name: snapshot.project, id: snapshot.project_id ?? null });
    if (refusal) throw new Error(refusal);
    const { folder: projectDir, renamedFrom } = projectFolder(dir, snapshot);
    const before = readStored(projectDir);
    const after = snapshot.timelines.map(storedTimeline);
    if (before.length === 0 && after.length === 0 && !renamedFrom) return null;
    // Histories from before project.json get it with their next save.
    const notedId = before.length > 0 && !!snapshot.project_id && readProjectId(projectDir) !== snapshot.project_id;
    writeStored(projectDir, snapshot, after);

    const relative = (folder: string) => path.relative(dir, folder).replaceAll("\\", "/");
    const pathspec = [relative(projectDir), ...(renamedFrom ? [relative(renamedFrom)] : [])];
    await gitOk(dir, ["add", "-A", "--", ...pathspec]);
    if ((await git(dir, ["diff", "--cached", "--quiet", "--", ...pathspec])).code === 0) return null;

    const changes = [
      ...(renamedFrom ? [`Resolve project renamed from "${path.basename(renamedFrom)}" to "${snapshot.project}"`] : []),
      ...describeChanges(before, after),
    ];
    if (changes.length === 0 && notedId) changes.push("Noted the Resolve project's id, so renaming it in Resolve keeps this history");
    const author = turn ? [`--author=${AGENT_NAMES[turn.provider]} <${turn.provider}@linker-edit.local>`] : [];
    await gitOk(
      dir,
      [...fallbackIdentity, "commit", "-q", "--no-verify", ...author, "-F", "-", "--", ...pathspec],
      commitMessage(changes, turn),
    );
    const commit = (await gitOk(dir, ["rev-parse", "--short", "HEAD"])).trim();
    return { commit, changes };
  }

  /** A timeline as Resolve has it now (current one without an id), or as it was in a save. */
  async function timeline(id: string | null, commit: string | null): Promise<PlayableTimeline | null> {
    await prepare();
    if (!commit) {
      const snapshot = await options.readSnapshot();
      for (const entry of mediaEntries(snapshot)) snapshotMedia.set(entry.id, entry);
      const raw = snapshot.timelines.find((candidate) => (id ? candidate.id === id : candidate.name === snapshot.current_timeline));
      return raw ? playableTimeline(storedTimeline(raw), snapshotMedia, null) : null;
    }
    // The commit comes from a URL; a value starting with "-" would be read as a git option.
    if (!/^[0-9a-f]{4,64}$/i.test(commit)) return null;
    const tree = await git(dir, ["-c", "core.quotePath=false", "ls-tree", "-r", "--name-only", commit, "--", HISTORY_ROOT]);
    if (tree.code !== 0) return null;
    const metadataFiles = tree.stdout.split("\n").filter((file) => file.split("/").length === 4 && file.endsWith("/metadata.json"));
    const metadata = await catFiles(dir, metadataFiles.map((file) => `${commit}:${file}`));
    const index = metadata.findIndex((text) => {
      try {
        return text !== null && (JSON.parse(text) as { id?: string }).id === id;
      } catch {
        return false;
      }
    });
    if (index === -1) return null;
    const folder = path.posix.dirname(metadataFiles[index]!);
    const files = await catFiles(dir, [...TIMELINE_FILES, "../media.json"].map((file) => `${commit}:${path.posix.join(folder, file)}`));
    const stored = parseTimelineFiles((file) => files[TIMELINE_FILES.indexOf(file)] ?? null);
    if (!stored) return null;
    const media = readMedia(dir);
    try {
      for (const entry of (JSON.parse(files.at(-1) ?? "") as { media: MediaEntry[] }).media) media.set(entry.id, entry);
    } catch {
      // Without that save's media.json, today's media list still finds most files.
    }
    return playableTimeline(stored, media, commit);
  }

  async function log(limit = 50): Promise<HistoryEntry[]> {
    await prepare();
    const result = await git(dir, [
      ...["-c", "core.quotePath=false", "log", "-n", String(limit), "--raw", "--no-renames"],
      ...["--format=%x1e%H%x1f%h%x1f%aI%x1f%an%x1f%ae%x1f%B%x1f", "--", HISTORY_ROOT],
    ]);
    if (result.code !== 0) return [];
    const commits = result.stdout
      .split("\x1e")
      .filter((entry) => entry.trim())
      .map((entry) => {
        const [hash = "", ...fields] = entry.split("\x1f");
        const raw = fields.pop() ?? "";
        // --raw lines end in a tab and the path; a timeline's files sit at resolve/<project>/<timeline>/<file>.
        const folders = new Set(
          raw
            .split("\n")
            .map((line) => line.split("\t")[1]?.split("/") ?? [])
            .filter((parts) => parts.length === 4)
            .map((parts) => parts.slice(0, 3).join("/")),
        );
        return { hash, folders: [...folders], entry: parseEntry(fields) };
      });

    // Each touched timeline before and after its commit, read in one git process.
    const names = commits.flatMap(({ hash, folders }) =>
      folders.flatMap((folder) => TIMELINE_FILES.flatMap((file) => [`${hash}^:${folder}/${file}`, `${hash}:${folder}/${file}`])),
    );
    const blobs = new Map((await catFiles(dir, names)).map((text, index) => [names[index]!, text]));
    const media = readMedia(dir);
    const isImage = (id: string) => mediaType(media.get(id)?.path)?.kind === "image";
    return commits.map(({ hash, folders, entry }) => {
      const side = (rev: string) =>
        folders
          .map((folder) => {
            try {
              return parseTimelineFiles((file) => blobs.get(`${rev}:${folder}/${file}`) ?? null);
            } catch {
              return null;
            }
          })
          .filter((timeline) => timeline !== null);
      return { ...entry, timelines: timelineStrips(side(`${hash}^`), side(hash), isImage) };
    });
  }

  async function recentContext(): Promise<string> {
    return (await log(5))
      .map(({ commit, date, author, subject, changes }) => {
        // %aI is local time with an offset, so its first 16 characters read as the user's clock.
        const when = date.slice(0, 16).replace("T", " ");
        const details = changes.length ? `\n${bullets(changes).replace(/^/gm, "  ")}` : "";
        return `${commit} ${when} ${author}: ${subject}${details}`;
      })
      .join("\n");
  }

  async function commitProjectFile(message: string) {
    const fallbackIdentity = await prepare();
    if (!fs.existsSync(path.join(dir, PROJECT_FILE))) return;
    await gitOk(dir, ["add", "--", PROJECT_FILE]);
    if ((await git(dir, ["diff", "--cached", "--quiet", "--", PROJECT_FILE])).code === 0) return;
    await gitOk(dir, [...fallbackIdentity, "commit", "-q", "--no-verify", "-F", "-", "--", PROJECT_FILE], `${message}\n`);
  }

  return {
    commitProjectFile(message) {
      const result = queue.then(() => commitProjectFile(message));
      queue = result.catch(() => {});
      return result;
    },
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
    log: (limit) => {
      const result = queue.then(() => log(limit));
      queue = result.catch(() => {});
      return result;
    },
    timeline: (id, commit) => {
      const result = queue.then(() => timeline(id, commit));
      queue = result.catch(() => {});
      return result;
    },
    mediaFile(id) {
      const file = (snapshotMedia.get(id) ?? readMedia(dir).get(id))?.path;
      const type = mediaType(file);
      return file && type && fs.existsSync(file) ? { path: file, type: type.type } : null;
    },
  };
}
