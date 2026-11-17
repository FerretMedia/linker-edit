# Plan: library, workspaces and series, and a T3 Code sidebar

Three things, built in separate threads.

**Naming:** the top level is called a **workspace** in the UI (a client, a brand, a channel: whatever you work for). A workspace holds **series** and projects. In code the type is `Collection` with kind `"workspace" | "series"`, and the UI words live in one place (`apps/desktop/src/lib/labels.ts`), so renaming later is one line.


1. **Library**: folders with the files you use again and again (music, sound effects, logos, LUTs…). One **universal** library for everything, and a library per **workspace**, **series** or **project** on top of it. The agent can search it and bring files into Resolve.
2. **Workspaces and series**: groups above projects. A workspace (a client, brand or channel) holds series and projects; a series holds episodes (projects). Each group has its own library, editing rules and look.
3. **A sidebar like T3 Code**: no more Resolve-first project list. Threads first, pinned threads, project icons, a few recent threads per project with "Show more", and projects grouped by workspace the way T3 Code groups them by repository.

## Where we are (main, 78a23d0)

- `Sidebar.svelte` lists projects as Resolve projects: a disk/network/cloud icon, an "Add *X*, open in Resolve" banner, every thread under each project, archived threads per project.
- A project is a folder with `linker-edit.json` (committed) and `.linker-edit/local.json` (this machine). `~/LinkerEdit/projects.json` lists them.
- `EDITING_RULES` in `apps/desktop/server/rules.ts` is one block of rules for every project. Nothing per client or series.
- `import_media` imports absolute paths into the media pool. ffmpeg ships with the app (`findFfmpeg`), so reading durations and loudness needs nothing new.
- The settings page (`#settings/<section>`, PR #17) is the pattern for a full page: the library page works the same way.

## The model

```
Universal library            (every project)
└─ Workspace "Ferret Media"      library + rules + look
   ├─ Series "Podcast"       library + rules + look
   │  ├─ Project "Ep 12"     library + rules
   │  └─ Project "Ep 11"
   └─ Project "Launch video"
Project "Wedding Jan"        (no workspace)
```

A project gets everything above it: the universal library plus its workspace's plus its series' plus its own. Rules stack the same way, most specific last.

### Workspaces and series ("collections")

One type, `Collection`, with a `kind` so the UI can say "workspace" or "series":

```ts
interface Collection {
  id: string;
  kind: "workspace" | "series";
  name: string;
  parentId: string | null;   // a series can sit in a workspace; one level deep is enough
  icon: ProjectIcon;         // T3's: lucide icon, emoji or monogram, plus a color
  rules: string;             // style guide in plain words: "lower thirds in Inter Bold, no swearing, end on the logo"
  library: LibraryFolder[];
  musicOnlyFromHere: boolean; // licensing: this workspace may only use its own music (see Library)
}
```

On disk, next to the projects:

```
~/LinkerEdit/
  library.json                    # the universal library's folders
  collections.json                # list: id, folder, order
  collections/<slug>/
    collection.json               # kind, name, parent, icon, library folders, flags
    RULES.md                      # the style guide, editable in the app or any editor
  projects/<slug>/linker-edit.json  # + "collection": { "id", "name" }
```

The project's `linker-edit.json` names its collection, so a teammate who gets the project later (see `sync-plan.md`) sees which workspace it belongs to.

**Styles** (decision 2): a style is *not* a third kind of group. A workspace or series already has rules and a look. A style is a **preset you attach**: rules text plus preferred library tags (e.g. "fast cuts, punchy SFX, upbeat music"). A project or collection can use one. It comes last (thread 5), only if workspaces and series don't already cover it.

### Library

A library is a list of **folders you point at**, never copies. Your music stays in `D:\Music`, Linker-Edit only remembers the folder and what kind it is.

```ts
interface LibraryFolder {
  id: string;
  kind: "music" | "sfx" | "graphics" | "luts" | "footage" | "other";
  label: string;            // "Epidemic downloads", "Brand logos"
  path: string;             // this machine's path; kept in local files, never committed
  note: string;             // "licensed for YouTube only", "use for intros"
}
```

- **Index**: a scan per folder (on add, on app start, and with a Rescan button) reads name, size, date, and with ffmpeg the duration, channels and loudness (`ebur128`). Kept in `~/LinkerEdit/cache/library/<folder id>.json`, refreshed only for changed files. No database.
- **Per file**: tags, a star, and a note, edited in the library page. "Used in" is filled by the import tool (which projects and timelines used this file), so the agent can avoid the same song twice in a series.
- **Licensing**: `musicOnlyFromHere` on a workspace hides the universal music from its projects. Without it, the agent prefers the most specific library and falls back to the universal one.

**Agent tools** (on the `linker` MCP server):

- `library_search { kind?, query?, tags?, min_seconds?, max_seconds?, scope? }` → files with scope (universal / workspace / series / project), duration, loudness, tags, note, star, used in. Searching is filename + tags + note; nothing is sent anywhere.
- `library_import { files, bin? }` → `import_media` into a `Library/<Kind>` bin, reusing a media pool item that is already there (same file path), and records "used in".
- Rule added to `EDITING_RULES`: "For music, sound effects and graphics, look in the library first (library_search). Don't search the disk for them. Respect folder notes about licensing."

**Playing files**: the library page plays audio in place. The HTTP server serves **only** files inside library folders (checked after resolving the path) and only to the owner, because remote access (Tailscale, later sync) can reach this server.

## The T3 Code sidebar

What T3 Code's current sidebar has that ours doesn't (its legacy sidebar is roughly what we have now):

| T3 Code | Linker-Edit now | Plan |
|---|---|---|
| **New thread** at the top, picks the project you're in | per-project pencil only | top button + `Ctrl+N`; asks for a project only when there's none selected |
| Search threads | none | search box: titles first, later message text |
| **Pinned** threads at the top | none | pin / unpin in the thread menu |
| Project icon: lucide, emoji or monogram + color | Resolve library icon (disk / server / cloud) | T3's icon picker in project settings; the Resolve kind moves to the tooltip and project settings |
| Shows the newest ~6 threads, **Show more** | all of them | 6, then "Show more" |
| **Settle** finished threads, **snooze**, unread dot | archive only | settle (drops out of the list, stays searchable), unread dot when a turn ends while you look elsewhere; snooze later |
| Projects grouped by repository, sorted by last activity | flat, in the order added | grouped by **workspace → series**, sorted by last activity; "No workspace" at the end |
| Pinned projects, archived projects | none | pin / archive a project |
| Settings and account at the bottom | same | same, plus **Library** |

Resolve-specific things leave the list: the "Add *X*, open in Resolve" banner becomes a small row under the top button only when Resolve has a project open that no Linker-Edit project has, and the green "open in Resolve" dot stays on the project row.

Sketch:

```
[logo] Linker-Edit                 2026.11.15
[ ✎ New thread ]                  [ 🔍 ]
Pinned
  📌 Intro cut v3                       2h
Ferret Media                            ⌄
  🎙 Podcast
     Ep 12 · rough cut                  ●
     Ep 11 · shorts                    1d
     Show 4 more
  🚀 Launch video                       ●
     Color pass                        3d
No workspace
  💍 Wedding Jan
     First assembly                    1w
─────────────────────────────────────────
♫ Library
⚙ Settings          Resolve ● connected
```

Thread 1 copies T3 Code's sidebar itself (decision 3); the table is the minimum, T3 Code is the reference where they differ.

## Threads

Each thread is its own T3 Code thread in its own worktree and becomes its own PR. Threads 1, 2 and 3 start together from `main`; 4 starts when those three are merged; 5 waits until the library has been used for real.

```
main ─┬─ 1. Sidebar like T3 Code ───────┐
      ├─ 2. Workspaces & series      ────┼─ 4. Put it together ── 5. Smart library / styles (later)
      └─ 3. Library (core + tools) ─────┘
```

To keep the parallel threads from fighting over files:

- `apps/desktop/shared/protocol.ts`: every thread **only adds** types and messages, in its own block with a comment header, so merges are additions.
- `Sidebar.svelte`: owned by thread 1. Threads 2 and 3 don't touch it (3 adds its Library button in thread 4).
- `apps/desktop/server/rules.ts`: thread 2 adds rule stacking, thread 3 adds one library rule line. Small, easy merge.
- `apps/desktop/server/app.ts`: each thread adds its own handlers in its own function, wired in with one line.

### Thread 1: Sidebar like T3 Code

- Branch `feat/t3-sidebar` from `origin/main`.
- Goal: the table and sketch above, **without** workspace grouping (projects sorted by last activity; grouping arrives in thread 4).
- Server: `pinnedAt`, `settledAt`, `lastReadAt` on `SavedThread`; `pinned`, `archived`, `icon` on the project. Messages `pin_thread`, `settle_thread`, `mark_read`, `update_project { icon, pinned, archived }`. Old files load with the new fields empty.
- UI: split `Sidebar.svelte` into `SidebarThreadRow`, `SidebarProject`, `SidebarSearch`, `IconPicker`. New thread on top, `Ctrl+N`, search, Pinned, 6 + Show more, settle and unread, project icon picker in project settings.
- Tests: store round-trip of the new fields, old-file loading, sorting by activity, an e2e for pin → settle → search finds it.
- Exit: 3 projects with 10+ threads each feel like T3 Code: find any thread in two clicks or by search.

### Thread 2: Workspaces and series

- Branch `feat/collections` from `origin/main`.
- Goal: the `Collection` model and its management, without a new sidebar.
- Server: `apps/desktop/server/collections/store.ts` (files as above, written like `projects/store.ts`: whole-file writes, bad files kept aside), `collection` in `linker-edit.json`. Messages `create_collection`, `update_collection`, `delete_collection` (only when empty), `move_project { projectId, collectionId | null }`.
- Rules: `rules.ts` builds the agent's rules as editing rules + workspace `RULES.md` + series `RULES.md` + project notes, each under its own heading, so the agent knows where a rule comes from.
- UI: a **Workspaces & series** settings section (list, create, rename, icon, rules editor) and a "Workspace / series" picker in project settings.
- Tests: store, nesting only one level, deleting a non-empty collection is refused, rule stacking order.
- Exit: put two projects in a series inside a workspace, write a workspace rule, and the agent follows it in both.

### Thread 3: Library

- Branch `feat/library` from `origin/main`.
- Goal: the universal and per-project library, its page, and the agent tools. Workspace and series libraries come in thread 4 (the code takes a list of scopes, so that's wiring only).
- Server: `apps/desktop/server/library/` with `folders.ts` (universal in `library.json`, project folders in `local.json`), `scan.ts` (ffmpeg duration + loudness, cache per folder, only changed files), `search.ts`, `tools.ts` (`library_search`, `library_import`), and the file route for playback, limited to library folders.
- UI: `#library` page like the settings page: scope tabs (Universal / This project), folders with kind and note, a file list with search, kind filter, play button, star, tags, note.
- Rule line in `EDITING_RULES` for the library.
- Tests: scan cache reuse, search filters, the file route refuses paths outside library folders (including `..` and symlinks), `library_import` reuses an existing media pool item (fixture Resolve).
- Live check: import a song into a real project's `Library/Music` bin.
- Exit: "put an upbeat song under this timeline and a whoosh on every cut" works from the library alone.

### Thread 4: Put it together

- Branch `feat/library-collections` from `origin/main` **after 1, 2 and 3 are merged**.
- Sidebar groups projects by workspace → series (collapsible, with their icons), drag a project onto a workspace or series, "No workspace" group, new project / new series from a workspace's menu.
- Workspace and series libraries: a scope tab per level on the library page, `library_search` searches the project's whole chain, and `musicOnlyFromHere`.
- Library button in the sidebar footer.
- Exit: a new episode in a series starts with the series' music, the workspace's logos and both sets of rules, with no setup.

### Thread 5 (later, only after real use): smarter library and styles

- Tempo (BPM) and mood tags computed locally, a waveform in the file list, "suggest tags" from file names and notes (Haiku, text only).
- "Used in" across a series: the agent avoids repeating a song in consecutive episodes.
- Styles as presets (decision 2), if workspaces and series don't cover it.
- Library folders shared with teammates once sync (`sync-plan.md`) exists.

## Decisions (2026-11-15)

1. **Naming**: workspace → series → project.
2. **Styles**: a preset you attach (thread 5), not a third kind of group.
3. **The sidebar**: copy T3 Code's UI as closely as we can, settle included. T3 Code is MIT (see NOTICE): copy from its public source where the sidebar is there, and match the installed app by eye where it isn't. Never copy code from the installed app's bundle.
4. **Library folder kinds**: music, sound effects, graphics, LUTs, footage, other.
5. **Copy or point**: point at existing folders only, for now.
