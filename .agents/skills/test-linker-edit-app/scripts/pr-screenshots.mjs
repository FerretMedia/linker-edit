// Uploads screenshots for a pull request and prints the Markdown to paste into
// its body. They go on a branch of their own, `pr-assets/<name>`, never on the
// PR's branch, so they aren't merged.
//
//   node pr-screenshots.mjs <name> <file.png>...
//   node pr-screenshots.mjs preview-panel before.png after.png
//
// The links are `github.com/<repo>/raw/<commit>/…`, which work while the repository
// is public and, for a signed-in browser, when it is private; `raw.githubusercontent.com`
// links of a private repository answer 404 to everyone viewing the PR.
// The commit, not the branch, is in the link, so it never points at another file.
// Running it again with the same name adds to the branch; a file of the same
// name is replaced, and the old links keep showing the old file.
import { execFileSync } from "node:child_process";
import { basename } from "node:path";

const [name, ...files] = process.argv.slice(2);
if (!name || files.length === 0) {
  throw new Error("Usage: node pr-screenshots.mjs <name> <file.png>...");
}

const git = (...args) => execFileSync("git", args, { encoding: "utf8" }).trim();

const remote = git("remote", "get-url", "origin");
const repo = remote.match(/github\.com[:/](.+?)(?:\.git)?$/)?.[1];
if (!repo) throw new Error(`origin is not a GitHub repository: ${remote}`);

const branch = `refs/heads/pr-assets/${name}`;
const tip = git("ls-remote", "origin", branch).split("\t")[0] || null;

// The branch's files so far, then these on top.
const entries = new Map();
if (tip) {
  git("fetch", "--quiet", "origin", branch);
  for (const line of git("ls-tree", tip).split("\n").filter(Boolean)) {
    entries.set(line.split("\t")[1], line);
  }
}
for (const file of files) {
  const blob = git("hash-object", "-w", file);
  entries.set(basename(file), `100644 blob ${blob}\t${basename(file)}`);
}

const tree = execFileSync("git", ["mktree"], {
  input: [...entries.values()].join("\n") + "\n",
  encoding: "utf8",
}).trim();
const commit = git(
  "commit-tree",
  tree,
  ...(tip ? ["-p", tip] : []),
  "-m",
  `Screenshots for the ${name} PR`,
);
git("push", "--quiet", "origin", `${commit}:${branch}`);

for (const file of files.map((path) => basename(path))) {
  console.log(
    `![${file.replace(/\.\w+$/, "")}](https://github.com/${repo}/raw/${commit}/${file})`,
  );
}
