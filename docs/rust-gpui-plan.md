# Linker-Edit in Rust with a GPUI window — plan

Status: plan, 2026-11-18. Nothing below is built yet. Rust isn't installed on the
dev machine yet (`cargo` not found).

## Goal

Make the desktop app lighter and faster: less memory, quicker start, a smoother window
(long chats, the sidebar, the timeline strip). We do it by replacing Electron + Node with
one native Rust program, and the Svelte window with [GPUI](https://www.gpui.rs/), Zed's
GPU-drawn UI framework.

## What gets faster, and what doesn't

Speed claims need to be honest before we spend weeks on this.

| Part | Today | After the port | Gain |
|---|---|---|---|
| App window | Electron (Chromium + Node), ~200–400 MB | One GPUI process, ~40–80 MB expected | **Large** |
| Start-up | Electron, then Node server, then SPA load | One binary, first frame in well under a second | **Large** |
| Long chats, sidebar, strips | DOM + Svelte | GPU-drawn, virtual lists | **Noticeable** with long threads |
| Server state, `/ws`, history diff, library scan | Node | Rust (tokio) | Small: rarely the bottleneck |
| Agent turns | Claude Code CLI / Codex processes, network-bound | **Unchanged**: same CLIs | None |
| Resolve calls | Python scripts in Resolve via ResolveMCP, 1 at a time | **Unchanged**: Python must run inside Resolve | None |
| Transcription | whisper.cpp process | Unchanged (whisper-rs in-process is optional later) | None |

So: the port makes **the app itself** cheap and snappy. Waiting on the agent or on Resolve
won't change. If the main pain is "a turn feels slow", the port won't fix it; profiling
the Resolve script calls would. Worth measuring the Electron app's memory and start time
first (Phase 0) so we can show the gain.

## What we have (apps/desktop, ~24k lines)

- `server/` (~14k lines TS): state, `/ws` + `/api` (`http.ts`, `app.ts` is 1.3k lines),
  providers (Claude Agent SDK, Codex SDK), Resolve over ResolveMCP stdio + 20-ish Python
  scripts, Git history, library, collections, transcription, remote access (Tailscale),
  Linker Connect (cloudflared), Linker login, study.
- `src/` (~8k lines Svelte): chat, tool cards, approvals, sidebar, settings, library,
  history panel, **timeline player** and **preview panel** (HTML `<video>`/`<audio>`).
- `desktop/`: Electron window, frameless title bar, electron-updater.
- `shared/protocol.ts` (800 lines): every message on `/ws`. The iPad app (#30) and
  Connect bearer clients (#28) speak it too.
- `packages/drp`: .drp/.drt reader/patcher in TS.

## Constraints the port must keep

1. **The server owns the state.** The GPUI window is one client of the core, like the
   browser, the iPad and a remote computer. Nothing is enforced only in the window.
2. **Browser, remote access and the iPad keep working.** They need HTTP + `/ws`. GPUI
   can't render in a browser, so the core keeps serving `/ws` and `/api` (see Decision 1).
3. **Guardrails stay in code**: `policy.ts` refusals, the Claude hook, `AI - …` timelines
   only, no `*CloudProject`/`CLOUD_*` (the guard test moves to Rust).
4. **Same data on disk**: `~/LinkerEdit` config, projects, threads, history repos,
   transcripts. Users switch from the Node app with no migration, and back.
5. **Windows first**, then macOS and Linux.

## Architecture

One Cargo workspace in the monorepo, next to the existing npm workspaces:

```
crates/
  protocol/   serde types for everything on /ws (port of shared/protocol.ts);
              ts-rs generates protocol.ts so Svelte + tests share one source
  core/       state: projects, threads, turns, approvals, collections, library, settings;
              an event bus of ServerMessage that every client subscribes to
  resolve/    ResolveMCP client over stdio (rmcp), runScript, locate, render, media;
              the Python scripts stay as they are
  providers/  claude (spawns `claude` with stream-json, does canUseTool + hook),
              codex (spawns `codex app-server` / `exec --json`), replay
  history/    edit history via the git CLI (same as today; gix later if worth it)
  mcp/        the `linker` MCP server the agents get (rmcp), run as `linker-edit mcp`
  drp/        port of packages/drp (zip, zstd, quick-xml)
  server/     axum: /ws, /api, static files for the web UI, Tailscale gate, Connect
  app/        the GPUI window + `main`; also `linker-edit serve` (headless) and `mcp`
```

The window talks to `core` through an in-process channel carrying **the same
`ClientMessage`/`ServerMessage` types** as `/ws`. That keeps one contract, makes the
window testable like any client, and lets the window later attach to **another
computer's** core over `/ws` (the computer picker) with no extra code path.

### Key crates

| Need | Crate | Note |
|---|---|---|
| Window/UI | `gpui` (git, pinned commit) + `gpui-component` | crates.io `gpui` 0.2 is a frozen Oct 2025 snapshot; pin a Zed commit instead |
| Async | `tokio` | GPUI has its own executor; bridge with channels |
| HTTP/WS | `axum`, `tokio-tungstenite` | |
| MCP client + server | `rmcp` (official Rust MCP SDK) | ResolveMCP client and the `linker` server |
| Markdown | `pulldown-cmark` → GPUI elements, or gpui-component's markdown view | replaces `marked` |
| Video/audio | `ffmpeg` process or `ffmpeg-next` + `cpal` | see "The hard parts" |
| Updates | `velopack` | Windows/macOS/Linux installers + delta updates, has a Rust SDK |
| Git | git CLI | as today |
| Types to TS | `ts-rs` | keeps the Svelte UI and iPad in step |

## The hard parts

1. **No official Rust Agent SDK.** The TS SDK is a wrapper that spawns Claude Code and
   speaks stream-json over stdio. We write our own small driver (~500 lines): spawn
   `claude --input-format stream-json --output-format stream-json`, the control
   requests for `canUseTool` and hooks, MCP config, resume, interrupt. Community crates
   exist ([claude-agent-sdk-rs](https://docs.rs/crate/claude-agent-sdk-rs/latest),
   [ccwire](https://docs.rs/crate/ccwire/0.1.2)) to read, but we own the code because
   approvals and the guardrail hook are the contract. We also have to **ship or find the
   `claude` binary** ourselves; today the npm SDK brings it. Same for **`codex`**.
   *Proof needed first* (Phase 1).
2. **Video playback.** GPUI has no `<video>`. The timeline player plays clips in sync with
   audio and applies looks; the preview panel plays renders. Plan: decode with ffmpeg
   (already bundled) into frames, upload them as GPUI images, play audio with `cpal`,
   and sync on the audio clock. Looks become a shader or ffmpeg filter. Fallback while
   building it: "Open in player". A WebView (gpui-component's Wry view) is possible on
   Windows/macOS but draws over the GPUI window and isn't on Linux, so not the plan.
3. **GPUI's maturity.** No stable API; breaking changes between Zed commits; Windows is
   the youngest backend ([source](https://repo.hex.pm/preview/gpui_native/0.2.0-rc.2/CHANGELOG.md)).
   Pin one commit, upgrade on purpose. Text inputs, IME, accessibility and file dialogs
   are thinner than in a browser; gpui-component covers inputs, lists, dialogs, menus.
4. **Two UIs.** Browser + remote access keep the Svelte UI unless we drop them
   (Decision 1). That's the price of a native window and it goes against "keep it small".
5. **Distribution.** `npx linker-edit` today. In Rust: installers (Velopack) as the main
   way; npm can keep working with per-platform binary packages, like Codex and Biome do.

## Phases

Each phase ends with a PR to `main` and something a user could run. Strangler order:
the Node app keeps shipping until the Rust one covers it.

**Phase 0 — Measure and set up (1–2 days).** Install Rust (rustup, MSVC build tools,
Windows SDK). Measure today's app: RAM idle and after a 300-message thread, start-up to
first frame, input lag in the composer. Spike: a GPUI "hello" window with our frameless
title bar on Windows. Go/no-go on the numbers.

**Phase 1 — Provider spikes (3–5 days).** In Rust, run one Claude turn and one Codex turn
with the Resolve and linker MCP servers attached, a `canUseTool` approval and the
guardrail hook refusing a non-`AI - …` timeline. Record both as fixtures. This is the
riskiest part; if it doesn't work cleanly, stop here.

**Phase 2 — Rust core behind the Svelte UI (3–5 weeks).** `protocol`, `core`, `resolve`,
`providers`, `history`, `mcp`, `server`. Port the server module by module, with its
tests. Generate `protocol.ts` from Rust. The existing Svelte UI, Playwright e2e and the
iPad app run unchanged against the Rust server: that's the proof the port is right.
Electron starts the Rust binary instead of Node for a while. Ship it.

**Phase 3 — GPUI window, chat first (3–4 weeks).** Sidebar (projects, threads,
collections, search, context menu), chat with markdown, tool rows, approval cards,
composer, context meter, Resolve status pill, settings. Behind a "new window" switch so
both can be used side by side.

**Phase 4 — GPUI for the rest (3–4 weeks).** History panel and clip strips, library page,
project/collection dialogs, folder picker, computer picker, remote access + Connect
settings, update card. Then the timeline player and preview panel (ffmpeg + cpal).

**Phase 5 — Drop Electron (1–2 weeks).** Velopack installers and updates for Windows,
macOS, Linux in `release.yml`; the existing `v2026.MM.DD` names. Remove `desktop/` and
the Node server. Update README, RELEASING.md, AGENTS.md.

Rough total: **3–4 months** of focused work, most of it Phases 2–4. The project stays
usable throughout.

## Testing

- Port the server's Vitest tests (40 files) to `cargo test` alongside each module; the
  stand-in Resolve (`fixtures/DaVinciResolveScript.py`) and recorded turns stay.
- A `/ws` contract test: the same recorded sessions against the Node and Rust servers
  during Phase 2 must produce the same `ServerMessage`s.
- Playwright e2e keeps covering the web UI. The GPUI window gets GPUI's `TestAppContext`
  tests for flows (approve, deny, restore) and screenshots for visual changes.
- The Cloud guard test (`*CloudProject`, `CLOUD_*`) moves to Rust before the TS one goes.

## Decisions for you

1. **Browser and remote access.** (a) Keep the Svelte UI for browser/remote, GPUI for the
   desktop window — recommended, nothing breaks, two UIs to maintain; (b) GPUI only:
   remote access becomes "GPUI app on the other computer connects over `/ws`", no
   browser/phone access; (c) keep Svelte everywhere and only replace Node + Electron's
   Chromium with a Rust core + system WebView (Tauri): most of the memory gain, little UI
   work, but no GPUI.
2. **npm.** Keep `npx linker-edit` via per-platform binary packages, or installers only?
3. **The `claude` and `codex` binaries.** Bundle them in the installer, or require the user
   to have Claude Code / Codex installed and find them?
4. **Order.** Core first (Phase 2 before 3, recommended: lowest risk, ships value early),
   or the GPUI window first on top of the Node server for a quicker visible result?

## Out of scope

- The iPad app (stays SwiftUI; it only needs `/ws` and Connect to keep working).
- Rewriting the Python Resolve scripts: they run inside Resolve and can't be Rust.
- Blackmagic Cloud: the same rules as today.
