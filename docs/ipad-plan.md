# Linker-Edit on iPad — plan

Status: plan, 2026-11-17. Nothing below is built yet. Research came from four parallel
agents (Resolve for iPad, the .drp format, Blackmagic Cloud, the iPad client); their
sources are linked inline.

## Goal

1. Chat with the agent and drive edits **from the iPad**, like on the desktop.
2. Open any Linker-Edit project in **DaVinci Resolve for iPad** in one tap, including
   pulling timelines/clips **from other projects** without juggling Resolve by hand.

## The hard limit, and how we get around it

DaVinci Resolve for iPad has **no scripting, no Workflow Integrations, no ResolveMCP, no
Shortcuts/App Intents**, and iPadOS doesn't let one app drive another. Nothing can reach
inside Resolve for iPad without a jailbreak, so we don't try. Instead there are four
bridges:

| Bridge | What it does | Status |
|---|---|---|
| **A. Remote brain** | A **native SwiftUI iPad app** is a client of the desktop Linker-Edit host. The agent edits in *desktop* Resolve (full scripting), the iPad shows chat, tool cards and previews. | Decided 2026-11-17: native SwiftUI, not a PWA or Capacitor |
| **B. Cloud, by hand** | *You* move the project into a Blackmagic Cloud library in Resolve's Project Manager and open it on the iPad. Linker-Edit never automates any of it (see "Blackmagic Cloud rules"); it only shows the steps and recognises the project once you've opened it. | Manual only |
| **C. File handoff** (primary) | Our own **.drp/.drt reader/patcher** (from the files Resolve exports) + proxy bundle, handed to the iPad via Files/share sheet. Works offline, no cloud. | Reader works on samples; iPad import untested |
| **D. Hardware remote** (optional) | An ESP32 paired to the iPad as a Bluetooth keyboard, typing Resolve shortcuts on command from our app. | Prototype idea only |

## What the research found

**Resolve for iPad** (v21.2, 2026-11-05, [App Store](https://apps.apple.com/us/app/davinci-resolve-for-ipad/id1581363826)):
- Pages: Cut, Color, Media, Photo. No Edit/Fusion/Fairlight officially.
- .drp/.dra project files move both ways with desktop; .drt and FCPXML import/export per
  release notes. AAF probably, EDL/OTIO unknown.
- Blackmagic Cloud project libraries + multi-user timelines work; 20.0+ has proxy-only sync.
- Speed Editor and Micro Color Panel over Bluetooth; Magic Keyboard with custom shortcuts.
- **Unknown:** whether it appears in "Open in…" for .drp/.drt/.fcpxml, any URL scheme.
- Desktop (21.1.2) and iPad must stay on the same major version: older Resolve refuses newer .drp.

**.drp/.drt format** (reverse engineered from sample files, reader in [`docs/ipad/drp-dump.mjs`](ipad/drp-dump.mjs)):
- Plain ZIP. `project.xml`, `MediaPool/Master/MpFolder.xml`, `SeqContainer/<uuid>.xml` per
  timeline, `Gallery.xml`. XML mirrors Resolve's DB: `<Type DbId="uuid">`, links by DbId.
- Version stamp per XML: `<!--DbAppVer="21.0.4.0005" DbPrjVer="17"-->`.
- Timeline clips: `Sm2TiVideoClip`/`Sm2TiAudioClip` with `Start`, `Duration`, `In`,
  `MediaRef`, `MediaFilePath`. Media file paths live in a zstd-compressed protobuf `<Clip>`
  blob in the media pool — that's what Resolve relinks by, not `MediaFilePath`.
- Effects, grades, markers, settings: hex-encoded protobuf / zstd / zlib blobs.
- **No checksums or signatures** beyond ZIP CRC. Blockers are the version gate, DbId
  consistency, and correct stored media metadata (Resolve doesn't re-probe on import).
- Verdict: **read = done, patch cuts = realistic, generate from scratch = clone templates.**
- Prior art (MIT): `samuelgursky/davinci-resolve-mcp` → `resolve-advanced/vendor/drp-format/`
  (local clone in `coding-projects/davinci-resolve-mcp`). Reuse with credit in NOTICE.

**Blackmagic Cloud** (local scripting `README.md` + `DaVinciResolveScript.pyi`):
- `ProjectManager.CreateCloudProject / LoadCloudProject / ImportCloudProject(path, settings) / RestoreCloudProject`.
- Settings: `CLOUD_SETTING_PROJECT_NAME`, `_PROJECT_MEDIA_PATH`, `_IS_COLLAB`, `_SYNC_MODE`
  (`CLOUD_SYNC_NONE / PROXY_ONLY / PROXY_AND_ORIG`), `_IS_CAMERA_ACCESS`.
- Per-clip status via `GetClipProperty("Cloud Sync")`.
- **No key to choose the library**; libraries are created only on the Cloud website.
- No public Blackmagic Cloud REST API. Its [terms](https://static.cloud.blackmagicdesign.com/html/terms)
  forbid reverse engineering it (17.3), using it "with the assistance of any automated scripting
  tool or software, other than as provided or expressly permitted by us" (31.5), and getting
  around its limits (31.12). So none of the calls above are used. See "Blackmagic Cloud rules".
- `open_project.py` on main called `LoadCloudProject`; the fix/cloud-terms PR removes it, so a
  cloud project is only recognised once you've opened it in Resolve yourself.

**iPad client** (repo read on origin/main b43861a):
- One WebSocket `/ws` + same-origin HTTP (`/preview`, `/media`, `/library`, `/api/timeline`),
  Range requests already work → video seeking works in Safari.
- Tailscale route works from iPad Safari today (owner check, no cookie).
- Linker Connect can't serve an iPad yet: only the desktop computer menu mints the session code.
- Electron-only code is just `src/lib/desktop.ts`, with fallbacks.
- **Not responsive:** fixed `w-64` sidebar (`Sidebar.svelte`), fixed `w-[32rem]` preview
  (`PreviewPanel.svelte`), hover-only actions, right-click-only menus, no manifest.
- Native builds need Xcode on macOS → GitHub Actions macOS runners + fastlane → TestFlight,
  plus the $99/yr Apple Developer Program.

## "Open projects from other projects"

Resolve has one open project per machine, which makes cross-project work painful. The .drp
reader removes that limit for *reading*:

1. **Browse any project without opening it.** The host reads each project's latest `.drp`
   backup (we already export one per turn into `<project>/backups/`) and lists its
   timelines, clips and media in the sidebar/library — on desktop *and* iPad.
2. **Pull from project X into project Y.** Pick a timeline (or a range of clips) from X;
   the host exports it as `.drt` from X's backup (or patches a `.drt` from the .drp), then
   `ImportTimelineFromFile` into the open project Y. Media pool items come along.
3. **Open on iPad** (one button per project, desktop and iPad UI):
   - **Local/network** project → "Send to iPad" builds the file bundle (bridge C): a .drp
     (or .drt/FCPXML) plus proxies, progress shown until it's ready to save to Files.
   - Project already **cloud**-linked → only text: "On the iPad, open Resolve → Project
     Manager → Blackmagic Cloud → *<name>*". Nothing is automated.

## Phase 0 — live checks (do these first, ~1 hour, needs you + the iPad)

The two biggest unknowns decide which bridge is primary. Nothing gets built on a guess.

- [ ] Update Resolve for iPad to 21.2 and confirm desktop 21.1.2 projects open (version gate).
- [ ] Export a .drp on desktop, save it to the iPad's Files, import it in Resolve for iPad's
      Project Manager. Then try the share sheet: does Resolve appear under "Open in…" for
      .drp, .drt and .fcpxml?
- [ ] Patch one clip's `Start` in that .drp by hand (I'll prepare the file), import on iPad.
      Proves a hand-edited .drp is accepted.
- [ ] Optional, all by hand in Resolve (no scripts): move a scratch project into a Blackmagic
      Cloud library with proxy-only sync and open it on the iPad. Tells us whether to describe
      this route in the "Send to iPad" dialog.
- [ ] Open Linker-Edit in iPad Safari over Tailscale (`https://<host>:8443`) and note what's
      broken.
- [ ] Optional: pair any Bluetooth keyboard to the iPad and check which Resolve shortcuts work
      on Cut/Color pages (bridge D's ceiling).

## Monorepo (decided 2026-11-17)

The repo becomes an npm-workspaces monorepo so the iPad app lives next to the desktop app:

```
apps/desktop   today's app: Electron shell, Node server, SvelteKit UI (npm package `linker-edit`)
apps/ipad      native SwiftUI iPad app (XcodeGen project, built on macOS CI runners)
packages/drp   @linker-edit/drp: .drp/.drt reader and patcher, zero dependencies
docs/          plans, shared by all apps
```

The conversion comes first (`chore/monorepo`); threads 1, 2 and 6 stack on it.

## Work split — agents

Each item is a subagent in its own worktree, one PR each, stacked on the monorepo PR.

| # | Branch | Bridge | Depends on | Scope |
|---|---|---|---|---|
| 0 | `chore/monorepo` | — | — | Move the app to `apps/desktop`, npm workspaces at the root, CI/release/auto-update paths, AGENTS.md and docs updated. |
| 1 | `feat/ipad-native` | A | 0 | Native SwiftUI iPad app in `apps/ipad`: connect to a host (Tailscale URL first), projects/threads sidebar (`NavigationSplitView`), streaming chat, tool cards, Allow/Deny approvals, Resolve status, preview player (AVPlayer). Swift package for protocol models + WebSocket client with unit tests; GitHub Actions macOS job builds and tests on the iPad simulator. No signing yet. |
| 2 | `feat/drp-format` | C | 0 | `packages/drp` (`@linker-edit/drp`): zero-dep reader (from `docs/ipad/drp-dump.mjs`), typed model (project, media pool, timelines, clips), patcher for cut-level edits that keeps blobs untouched, version-gate check. Fixtures from desktop exports. Vendoring the MIT drp-format library is allowed where it saves work. |
| 3 | `feat/project-browser` | — | 2 | "Browse any project without opening it" + "pull timeline from project X into Y" (.drt export/import, enable `.drt` in `import_timeline`'s `IMPORTABLE`). UI in sidebar/library. |
| 4 | — | B | — | **Dropped.** Automating Blackmagic Cloud would break its terms (31.5). Done instead: the `fix/cloud-terms` PR, which removes our only cloud call and adds a guard test. |
| 5 | `feat/ipad-bundle` | C | 2, Phase 0 file check | "Send to iPad": render proxies (`server/resolve/render.ts`), patched .drp (or .drt/FCPXML, whichever the iPad accepts best) pointing at proxy names, zipped bundle, `navigator.share({files})` / download from the iPad UI. |
| 6 | `feat/connect-native` (+ Linker PR later) | A | 0 | Linker Connect for the native app: bearer-session auth on `/ws` and HTTP, so the iPad app reaches a host without Tailscale after Linker sign-in (Clerk PKCE in `ASWebAuthenticationSession`). |
| 7 | `feat/ipad-release` | A | 1, 6 | Signing + TestFlight: GitHub Actions macOS + fastlane `match`/`pilot`, App Store Connect API key, APNs push for approvals/finished turns. Needs the Apple Developer account. |
| 8 | `hw/ble-remote` (separate repo) | D | Phase 0 keyboard check | Optional: ESP32-S3 BLE HID keyboard firmware + a GATT command channel; app sends "blade at playhead", "next clip", etc. Keystrokes only, no feedback from Resolve. |

**Wave 1:** 0, then 1, 2, 6 in parallel on top of it.
**Wave 2 (after Phase 0):** 3, 5.
**Wave 3:** 7, then optionally 8.

The iPad client is native SwiftUI (your call, 2026-11-17). The cost is a second UI to keep in
step with the web UI; the Swift protocol models are tested against JSON fixtures recorded
from the Node server so drift shows up as a failing test.

## Blackmagic Cloud rules (decided 2026-11-17)

You chose not to risk the [Blackmagic Cloud terms](https://static.cloud.blackmagicdesign.com/html/terms).
These are hard rules for every thread (also in `AGENTS.md`):

- No `CreateCloudProject`, `LoadCloudProject`, `ImportCloudProject`, `RestoreCloudProject` or
  `CLOUD_*` settings, from our scripts or the agent's (`server/policy.ts` refuses them; a test
  fails if a script names them).
- No requests to Blackmagic Cloud servers, no probing their endpoints, no automating their
  website, no reverse engineering the service.
- Cloud projects are moved, opened and synced by you, in Resolve. Linker-Edit only recognises
  one once it's open, and may show written steps.
- Reverse engineering applies to **files only**: .drp/.drt exports on your own disk.

## Decisions for you

1. **File handoff (C) is the primary path.** Cloud stays manual.
2. **Apple Developer account** ($99/yr) for thread 7 (TestFlight). Until then the app only
   builds and runs in the simulator on CI; you can't install it on the iPad without it.
3. **Speed Editor emulation: not planned.** Its Bluetooth link uses a challenge/response
   authentication (documented by [smunaut/blackmagic-misc](https://github.com/smunaut/blackmagic-misc));
   faking a genuine device means getting around that check. A plain Bluetooth keyboard (8)
   gets most of the value without it.

## Risks

- **Version drift**: iPad and desktop Resolve must move together; the drp model checks
  `DbPrjVer` and refuses mismatches with a clear message.
- **Undocumented format changes**: every Resolve update can change the XML/blobs. Fixtures
  per version + a `test:live` round-trip (export → patch → import on desktop) catch it.
- **Media size**: proxy bundles through a Cloudflare tunnel are slow/costly; prefer LAN/
  Tailscale for downloads, cloud sync for big projects.
- **Untested foundations**: Tailscale Serve, Linker Connect and cloud project opening are all
  still not live-tested (see their PRs). Phase 0 doubles as their first real test.
