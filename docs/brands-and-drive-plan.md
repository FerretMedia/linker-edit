# Plan: brand tree, Google Drive sync and storage per computer

## What changes

- **Brand tree.** The sidebar groups projects in a tree of brands, sub-brands and series. These shapes are allowed:
  - brand, project
  - brand, series, project
  - brand, sub-brand, project
  - brand, sub-brand, series, project

  Today a workspace can hold series and nothing nests deeper (`apps/desktop/server/collections/collections.ts:88`).
- **Google Drive sync.** Linker-Edit signs in to Google Drive itself and keeps brands and projects in a Drive folder. Each computer picks which projects it mirrors. A mirrored project has every file on local disk and syncs both ways. Any other project stays in the cloud and only shows in the sidebar. Google Drive for desktop is not used. It can only mirror all of My Drive, the user's My Drive holds 5 TB, and it has no API that pins a single folder offline on Windows.
- **Storage per computer.** A settings page lists every computer with its disks, free space and the space each project takes on it. Once Drive sync lands, it also shows how much of each project is local and how much is cloud only.
- **Add existing projects from Drive.** A computer browses the brand tree on Drive and mirrors a project with one click. A local project can be moved up to Drive.

T3 Code has no nested folders to copy. Its newest release (v0.1.12, PR #35925) lets one project cover several Git repos. Its sidebar only goes one level deep. Three pieces carry over. The folder header row (`apps/web/src/components/sidebar/SidebarProjectFolderHeader.tsx`), the per-device open and closed state (`uiStateStore.ts`, `projectExpandedById`), and the folder scan (`apps/server/src/workspace/WorkspaceFolders.ts:280`). The scan becomes "find Linker-Edit projects inside a brand folder on Drive".

## PRs in landing order

| PR | Branch | Builds on | Review before merge |
| --- | --- | --- | --- |
| 0 | `spike/drive-scope` (throwaway, not merged) | trunk | no |
| A | `feat/brand-tree` | trunk | yes, it changes the sidebar |
| B | `feat/computer-storage` | trunk | yes, new settings page |
| C | `feat/google-drive-signin` | 0's answer | no |
| D | `feat/drive-sync` | C | no, nothing new on screen besides sync state |
| E | `feat/resolve-handoff` | D | yes, it changes opening a project |
| F | `feat/brands-on-drive` | A and D | yes, it changes adding a project |
| G | `feat/storage-drive` | B and D | yes, it changes the storage page |

A, B and 0 start from trunk and run in parallel. C waits for the spike's answer on the OAuth scope. D, E, F and G stack on C. F is stacked on D and rebased onto trunk after A merges. G does the same after B merges.

Files each PR may touch:

- A: `apps/desktop/shared/protocol.ts` (collection types), `apps/desktop/server/collections/*`, `apps/desktop/server/projects/store.ts` (`ProjectCollection` only), `apps/desktop/src/lib/labels.ts`, `apps/desktop/src/lib/components/Sidebar.svelte`, `apps/desktop/src/lib/components/sidebar/*`, `CollectionEditor.svelte`, `CollectionsSettings.svelte`, `apps/desktop/e2e/collections.spec.ts`, the iPad fixtures if the snapshot shape changes.
- B: `protocol.ts` (storage messages), new `apps/desktop/server/storage/*`, `apps/desktop/server/app.ts` wiring, a new `ComputersStorageSettings.svelte`, `SettingsPage.svelte`.
- C: new `apps/desktop/server/drive/auth.ts`, `drive/client.ts`, settings row for the Google account.
- D: `drive/sync/*`, `projects/store.ts` (Drive link fields), `protocol.ts` (sync state).
- E: `apps/desktop/server/resolve*`, `drive/sync/*` hooks, the project-open path in `app.ts`.
- F: `collections/store.ts` (brand folders on Drive), `AddProjectDialog.svelte`, sidebar cloud-only rows.
- G: `storage/*`, `ComputersStorageSettings.svelte`.

## Decisions already made

- The user picked our own Drive sync over Google Drive for desktop (2026-11-26).
- The UI words are **Brand**, **Sub-brand** and **Series** (2026-11-26). They replace "workspace" everywhere in `labels.ts`.
- Blackmagic Cloud stays out. Its terms ban automated scripts (see `AGENTS.md`), so the Resolve project travels as a `.drp` file inside the project's Drive folder.
- One computer edits a project at a time. Resolve has one project open per computer already. Drive adds a lock that says which computer holds the project.

## Data shapes

Brand tree. The allowed parents live in one table, so a new level is one row, not another `if`:

```ts
type CollectionKind = "brand" | "subbrand" | "series";

const PARENT_KINDS: Record<CollectionKind, readonly (CollectionKind | null)[]> = {
	brand: [null],
	subbrand: ["brand"],
	series: ["brand", "subbrand"],
};

/** Root first: the brand, then the sub-brand and the series when there are any. */
function collectionChain(collections: Map<string, SavedCollection>, project: SavedProject): SavedCollection[];
```

A project's rules, lessons, skills and `autonomous` flag come from the chain in order, root first, then the project. Today's `collectionChain` returns `{ workspace, series }`. It becomes the array above, and every caller moves to it in the same PR. A `collection.json` with kind `"workspace"` is read as `"brand"` and written back as `"brand"` on its next save.

Drive link, in the project's committed `linker-edit.json`, so every computer sees the same Drive folder:

```ts
interface DriveLink {
	/** The project's folder on Drive. */
	folderId: string;
	/** Google's account id, so a computer signed in to another account doesn't touch it. */
	accountId: string;
}
```

This computer's side, in the gitignored `.linker-edit/local.json`:

```ts
type Mirror =
	| { state: "cloud" }
	| { state: "downloading"; done: number; total: number }
	| { state: "mirrored"; manifest: string }
	| { state: "error"; message: string };
```

Sync manifest, one per mirrored project at `.linker-edit/drive-manifest.json`:

```ts
interface SyncManifest {
	version: 1;
	/** Drive's changes.list page token for this project. */
	pageToken: string;
	/** Path relative to the project folder, with forward slashes. */
	files: Record<string, { driveId: string; md5: string; size: number; mtimeMs: number }>;
}
```

Storage, one report per computer:

```ts
interface ComputerStorage {
	computerId: string;
	disks: { path: string; label: string; totalBytes: number; freeBytes: number }[];
	projects: { projectId: string; localBytes: number; cloudBytes: number | null; mirror: Mirror["state"] | null }[];
	measuredAt: string;
}
```

What syncs and what doesn't:

| Path | Syncs | Why |
| --- | --- | --- |
| media, `resolve/*.drp`, `linker-edit.json`, `AGENTS.md` | yes | the project itself |
| `.linker-edit/threads/` | no | each computer already shows the other computers' threads through the home server |
| `.linker-edit/local.json`, `drive-manifest.json` | no | this computer only |
| `.git/` | no, see open questions | thousands of small files, and Drive has no atomic multi-file write |

## PR 0: spike on the Drive scope

The user has 5 TB already on Drive. "Add existing projects from Drive" only works when Linker-Edit can read folders it didn't create. Two scopes are possible:

- `drive.file` needs no Google review. The app sees only files it created and files the user picks in Google's Picker. Whether picking a folder also grants its contents is not proven.
- `drive` sees everything. Google calls it restricted. A public app needs Google's verification and a paid security assessment. An unverified app shows a warning screen and caps at 100 users. In "Testing" status the sign-in expires every 7 days.

- [ ] Depends on: the user creates a Google Cloud project with an OAuth client of type "Desktop app" and adds their account as a test user. The plan can't do this step for them.
- [ ] Files: `scripts/drive-spike.ts` only, on a throwaway branch.
- [ ] Builds: a script that signs in with each scope, opens the Picker on a folder made in drive.google.com, and lists that folder's children.
- [ ] Result: a table of what each scope can read. It covers a picked folder, files inside it, a subfolder, and a file added later through drive.google.com.
- [ ] Verification: run the script against the user's account and paste the table into this plan.
- [ ] Decision this settles: when `drive.file` plus Picker reads a picked folder's contents, C uses `drive.file`. Otherwise C uses `drive`, and the user decides between a verified app and "bring your own OAuth client".

## PR A: brand tree

- [ ] Depends on: trunk.
- [ ] Files: listed above.
- [ ] Builds: `CollectionKind` becomes `"brand" | "subbrand" | "series"`, with `PARENT_KINDS` deciding where each can go. `collectionChain` returns the whole chain. The sidebar draws the tree with T3's folder header row and per-device open and closed state. **Move to** on a project and on a collection offers every allowed parent. Old `workspace` files load as brands.
- [ ] Result: the user builds all four shapes from the top of this plan, and each project's agent gets the rules of every level above it, brand first.
- [ ] Unit: `parentRefusal` for every pair in `PARENT_KINDS` and a few that aren't allowed. `collectionChain` on all four shapes. Loading a `workspace` file gives a brand. Deleting a sub-brand that still holds a series is refused.
- [ ] Live: in the Electron app, make brand "Ferret Media", sub-brand "Shorts", series "Weekly" and one project in each level. Restart the app. Pass when the tree, open and closed state, and the project's brief (`read_skill` in a turn) show all three levels in order.
- [ ] Perf: sidebar render time with 200 projects in 30 collections. Probe: Playwright `performance.now()` around the first paint of `Sidebar.svelte` with a seeded store. Measure trunk first. Fail above trunk plus 20 percent.

## PR B: storage per computer

- [ ] Depends on: trunk.
- [ ] Builds: each computer's server answers `storage` with a `ComputerStorage` report. Disks come from `fs.statfs` on every drive that holds a project, plus the app's own data folder. A project's size comes from a folder walk in a worker thread, cached per file by `mtimeMs` and size, so the second measure only stats files. The home server asks every computer, the same way `server/machines.ts` proxies other calls. **Settings > Computers and storage** shows one card per computer. Each card has a bar per disk and a list of projects sorted by size. An offline computer shows its last report greyed with the time it was measured.
- [ ] Unit: the walk on a temp folder with known sizes. The cache skips unchanged files. A computer that doesn't answer gives the cached report marked stale.
- [ ] Live: two computers, one reached through the home server. Pass when both cards show disk numbers that match Windows Explorer within 1 percent, and a project's size matches Explorer's "Size" for its folder.
- [ ] Perf: time to measure a project folder of at least 200 GB. Probe: the server logs both walks. Fail when the second walk takes more than 2 seconds.

## PR C: Google Drive sign-in

- [ ] Depends on: the scope that spike 0 picked.
- [ ] Builds: OAuth for a desktop app with PKCE and a loopback redirect on `127.0.0.1`. The refresh token is stored encrypted with Electron `safeStorage`. A thin client (`drive/client.ts`) covers list, get, create folder, resumable upload, ranged download and `changes.list`. Settings gets a **Google Drive** row with the account and **Sign out**.
- [ ] Unit: the client against a recorded fake Drive server. A 401 refreshes the token once. A 403 rate limit backs off.
- [ ] Live: sign in, restart the app, list My Drive's root. Pass when no second sign-in is asked.
- [ ] Perf: none, nothing hot.

## PR D: Drive sync engine

- [ ] Depends on: C.
- [ ] Builds: a project with a `DriveLink` syncs while it is mirrored on this computer. Startup compares the local folder with the manifest and Drive's changes feed. A local change uploads with a resumable upload in 8 MiB chunks. A remote change downloads with byte ranges into a temp file and renames it into place after the md5 matches. A file changed on both sides keeps both, with the other one named `<name> (from <computer>)<ext>`. Every step writes the manifest, so a killed app resumes where it stopped. The sidebar row shows a small sync state: up to date, syncing with a percent, or an error.
- [ ] Unit: the planner as a pure function of `(manifest, local scan, remote changes) -> actions`, with a table of cases. Cases cover new on each side, changed on each side, changed on both, deleted on each side, and renamed. A kill between chunk uploads resumes from Drive's reported offset.
- [ ] Live: mirror a 20 GB project on two computers. Add a file on one and see it on the other. Change the same small file on both while one is offline. Pass when the files match by md5 on both and the conflict copy exists.
- [ ] Perf: upload throughput on a 10 GB file. Probe: bytes over wall time in the sync log. Baseline: the same file uploaded through drive.google.com in Chrome on the same line, measured first. Fail below 70 percent of the baseline.

## PR E: Resolve project hand-off

- [ ] Depends on: D.
- [ ] Builds: a lock file `resolve/lock.json` in the project's Drive folder says which computer holds the project and since when. **Open** on a computer that doesn't hold the lock asks the holder to hand off when it is online, or offers to take over when it has been offline for a day. Hand-off exports the open Resolve project to `resolve/<name>.drp`, waits until the upload finishes, and moves the lock. Taking over downloads the `.drp`, imports it into this computer's local database, and relinks media to this computer's mirror folder with `MediaPool.RelinkClips`. Media on Drive is kept inside the project folder, so a relative path finds it on every computer.
- [ ] Unit: the lock as a state machine with states free, held, handing off and stale, tested on every move between them.
- [ ] Live: edit a timeline on computer 1, hand off, open on computer 2. Pass when the timeline has the same clips and every clip is online, with none shown red as offline.
- [ ] Perf: hand-off time for a project with a 50 MB `.drp`. Fail above 2 minutes on the user's line.

## PR F: brands and projects on Drive

- [ ] Depends on: A and D.
- [ ] Builds: Linker-Edit keeps the tree on Drive as folders, `Linker-Edit/<brand>/<sub-brand>/<series>/<project>`. Each collection folder holds its `collection.json` and `SKILL.md`, so brands and their rules reach every computer. **Add project** gets a **From Google Drive** tab. It lists the tree with T3's scan rule (a folder holding `linker-edit.json`, looked for two levels down), and **Mirror here** asks for a local folder and starts D. Cloud-only projects show greyed in the sidebar with a cloud icon. **Move to Drive** on a local project uploads it under its brand's folder.
- [ ] Unit: the scan on a fake Drive tree. Moving a project between brands moves its Drive folder too.
- [ ] Live: on computer 2, which has nothing local, open **Add project > From Google Drive**, mirror a project made on computer 1. Pass when it appears under the same brand with its rules, and the first turn's `read_skill` shows them.
- [ ] Perf: listing a Drive tree of 1,000 folders. Fail above 5 seconds.

## PR G: Drive numbers on the storage page

- [ ] Depends on: B and D.
- [ ] Builds: each project row on the storage page shows local and cloud-only bytes and its mirror state. **Stop mirroring** frees the local copy after a check that every file is uploaded.
- [ ] Unit: freeing is refused while the manifest has files that are not uploaded.
- [ ] Live: stop mirroring a 20 GB project. Pass when the disk's free space grows by about 20 GB and the project shows as cloud only on every computer.
- [ ] Perf: none beyond B's.

## Open questions

- Edit history in `.git` doesn't sync in this plan. Two ways out exist. One is a `git bundle` uploaded on hand-off. The other is to keep history per computer. Decide after E runs live.
- The scope in C depends on spike 0. When it lands on `drive`, the user picks between a verified Google app for every Linker-Edit user and each user bringing their own OAuth client.
- `ImportProject` on a computer that already has an older copy of the same project. Resolve may refuse the name or make a second project. Prove it on Resolve 21.1.2 before E's hand-off code.

## Still unproven

- Everything that touches Google. Nothing here has run against a real Drive account yet.
- `MediaPool.RelinkClips` across computers with different drive letters.
- Folder walk speed on a 5 TB tree. B measures one 200 GB project, not the whole Drive.
