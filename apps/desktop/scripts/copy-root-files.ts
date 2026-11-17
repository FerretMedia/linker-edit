// README, LICENSE and NOTICE live at the repository root; the npm package needs its own copies. Run by `npm pack` and
// `npm publish` as prepack. The copies are ignored by Git.
import * as fs from "node:fs";
import * as path from "node:path";

const app = path.join(import.meta.dirname, "..");
for (const file of ["README.md", "LICENSE", "NOTICE"]) fs.copyFileSync(path.join(app, "..", "..", file), path.join(app, file));
