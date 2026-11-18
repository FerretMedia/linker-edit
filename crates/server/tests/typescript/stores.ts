// The TypeScript side of typescript_cross_check.rs: writes the project, collection and history files of
// fixture.json with the TypeScript stores (`write <dir>`), or reads what the Rust ones wrote (`read <dir>`), and
// prints what the stores load as JSON. Remove with the TypeScript server.
import * as fs from "node:fs";
import * as path from "node:path";
import { createHistory } from "../../../../apps/desktop/server/history/history.ts";
import type { RawSnapshot } from "../../../../apps/desktop/server/history/model.ts";
import { fileCollectionStore } from "../../../../apps/desktop/server/collections/store.ts";
import { fileProjectStore } from "../../../../apps/desktop/server/projects/store.ts";

const fixture = JSON.parse(fs.readFileSync(new URL("./fixture.json", import.meta.url), "utf8"));
const [mode, dir] = process.argv.slice(2) as ["write" | "read", string];
const errors: string[] = [];
const onError = (error: unknown) => errors.push(String(error));

const projects = fileProjectStore({ root: path.join(dir, "projects"), legacyDir: path.join(dir, "workspace"), onError });
const collections = fileCollectionStore({ root: dir, onError });
let snapshot: RawSnapshot = fixture.snapshots[0];

if (mode === "write") {
  const project = projects.create(fixture.project);
  projects.saveThread(project, fixture.thread);
  projects.saveProject({ ...project, icon: fixture.icon, pinned: true });
  const workspace = collections.create(fixture.workspace);
  const series = collections.create({ ...fixture.series, parentId: workspace.id });
  collections.save({ ...workspace, lessons: [fixture.lesson] });
  collections.saveExample(series.id, "Reel 12 - Launch!.mp4", "# Reel 12\n");
  const history = createHistory({ dir: project.dir, readSnapshot: async () => snapshot });
  await history.record();
  fs.writeFileSync(path.join(project.dir, "PROJECT.md"), "# Promo\n");
  snapshot = fixture.snapshots[1];
  await history.record(fixture.turn);
}

const loaded = projects.load();
const promo = loaded.find(({ project }) => project.name === "Promo")!.project;
const history = createHistory({ dir: promo.dir, readSnapshot: async () => fixture.snapshots[1] });
const log = await history.log();
const first = log.at(-1)!.commit;
console.log(
  JSON.stringify({
    projects: loaded,
    collections: collections.load(),
    examples: collections.load().map((collection) => collections.examples(collection.id)),
    log,
    saved: await history.timeline("t2", first),
    live: await history.timeline(null, null),
    // A save of the same Resolve project finds nothing to save when the files are byte for byte the same.
    again: await history.record(),
    errors,
  }),
);
