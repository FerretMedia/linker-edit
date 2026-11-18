// README, LICENSE and NOTICE live at the repository root, and the Resolve scripts with the Rust server; the npm package
// needs its own copies. Run by `npm pack` and `npm publish` as prepack. The copies are ignored by Git.
import * as fs from "node:fs";
import * as path from "node:path";

const app = path.join(import.meta.dirname, "..");
for (const file of ["README.md", "LICENSE", "NOTICE"]) fs.copyFileSync(path.join(app, "..", "..", file), path.join(app, file));
const scripts = path.join(app, "..", "..", "crates", "server", "src", "resolve", "scripts");
fs.rmSync(path.join(app, "resolve-scripts"), { recursive: true, force: true });
fs.mkdirSync(path.join(app, "resolve-scripts"));
for (const file of fs.globSync("*.py", { cwd: scripts })) fs.copyFileSync(path.join(scripts, file), path.join(app, "resolve-scripts", file));
