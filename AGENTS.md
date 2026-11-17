# Linker-Edit

Linker-Edit lets Claude and Codex edit a DaVinci Resolve Studio project from a
chat, in an app modelled on T3 Code. A Node server (`server/`) runs the agents
and talks to Resolve through the MCP server Resolve ships with; a SvelteKit
single-page app (`src/`) is the UI; Electron (`desktop/`) is the window. Users
get it as the desktop app or with `npx linker-edit`.

This file is for any coding agent; `CLAUDE.md` only imports it. Its rules are
good defaults, and the developer's request can override any of them. If one
fights the task in front of you, say so before breaking it.

## Taste

Simple systems over impressive ones; the project is meant to stay small and
obvious. More code is fine; more complexity is not. Every new interface needs
something that varies or is remote behind it (Claude against Codex, the real
Resolve against the stand-in). Don't keep complexity because it already exists,
and fight scope creep: do what was asked, minimally and properly.

## Layout

An npm workspaces monorepo; run `npm` from the root. `apps/` holds the apps,
`packages/` shared libraries (`packages/drp` reads and patches .drp/.drt
files), `docs/` plans. The SwiftUI iPad app (`apps/ipad`) is checked only by
`ipad.yml` CI on a pushed branch: no Swift here. The app, published to npm as
`linker-edit`, is `apps/desktop`; paths here are in it:

- `server/`: all state (projects, threads, turns, approvals); `/ws` and `/api`
- `server/providers`: Claude and Codex sessions, and replaying a recorded turn
- `server/resolve`: the connection to Resolve, its tools, the Python they run
- `server/agents`: the `linker` MCP server the agents get
- `server/history`: the edit history, the Resolve project in Git, per turn
- `server/projects`: projects, threads, and their link to a Resolve project
- `src/`: the UI, built to `build/` and served by the server
- `shared/`: what both sides import; `protocol.ts` is everything on `/ws`
- `desktop/`: Electron, the window and the updater

- **The server owns the state.** The UI renders what arrives over `/ws` and
  sends commands back; a rule enforced only in the UI isn't enforced.
- **Node runs the TypeScript itself** (`node server/index.ts`): only erasable
  syntax (no enums, namespaces or parameter properties), imports end in `.ts`.
- **Guardrails live in code, not prompts.** What agents may not do (change a
  timeline not named `AI - …`, delete media, switch projects) is refused by the
  tools and the hook in both approval modes. The README's "How approvals work"
  is the contract; keep the two in step.
- **Pull requests go to `main`.** Only `.github/workflows/release.yml`, run by
  hand, ships; see `RELEASING.md`.

## The five ways to hurt yourself

1. **The developer's Resolve.** A server you start finds the installed Resolve
   and works on whatever project is open: real work. Never send a real agent
   turn against it, click _Open in Resolve_, _Use as main timeline_, _Full
   render_ or _Restore_, or run `npm run test:live`, unless asked.
2. **The developer's data.** `~/LinkerEdit` holds their projects, threads,
   settings, Linker sign-in and speech engine. A server you start uses it unless
   `LINKER_EDIT_CONFIG` points at a config of your own; always give it one.
3. **Killing by pattern.** The developer keeps `npm run dev` (5317 and 4317)
   and the desktop app running in the main checkout. Never kill those ports,
   `pkill node` or `taskkill /IM node.exe`; stop only what you started, by PID.
4. **Working in the main checkout.** Several agents share this machine. Work
   in a worktree; before committing, check `git log -1` and the branch.
5. **Shipping.** Never run the release workflow, `npm publish`, or push tags.

## Blackmagic Cloud

Its [terms](https://static.cloud.blackmagicdesign.com/html/terms) ban scripts
using the service (31.5) and reverse engineering it (17.3). Never call
`*CloudProject` or `CLOUD_*` (`apps/desktop/server/resolve/scripts.test.ts` and
`policy.ts` guard this), contact its servers, or open or upload cloud projects:
the user does that in Resolve. .drp/.drt files on disk are fair game.

## Worktrees

`t3.json` runs `npm ci` when T3 Code creates a worktree; run it yourself in a
worktree made any other way. `npm run dev` has fixed ports, so a worktree's app
runs from a build on a port of its own; `test-linker-edit-app` shows how.

## Hit every surface

The commonest defect is a change that works where it was tried and is missing
elsewhere. Before calling a change done, go through this list and say which
applied:

- **Claude and Codex.** A change to one provider: does the other need it? What
  Codex can't do (no hook, no context meter) is said in the UI and README.
- **Full access and Supervised.** Both approval modes; a refusal holds in both.
- **Where the Resolve project lives.** Local database, network (PostgreSQL),
  Blackmagic Cloud, or a project folder not linked yet.
- **Window, browser and another computer.** The UI runs in Electron, in a
  browser, and over remote access, where folders and Resolve are the edit
  machine's, not the viewer's.
- **Windows, macOS and Linux.** Paths, spawning and installs differ; most
  Resolve users are on Windows.
- **Restarts.** Thread state survives a restart, a resumed session and a
  switch between agents.
- **Ways out.** Added a way in? Add the way out: undo, archive, turn off.
- **Docs.** The README is the user's manual; `RELEASING.md`; this file.

## Verifying

- **While iterating**, run only the tests you touch: `npm test -- <file>`.
- **Before calling it done**, `npm run typecheck` and `npm test`, and
  `npm run test:e2e` when a flow in the UI changed.
- **Test what can break:** the guardrails and their refusals, the Resolve
  scripts (against the stand-in Resolve), the `/ws` protocol, the history and
  restore formats, config parsing, and Windows paths. Leave out what types
  already guarantee, copy text, and markup rendered from props.
- **Extend before you add.** Add a case to the test that already covers the
  behavior; write a new one only when none fits. Test at the lowest layer that
  fails, once.
- **A bug fix's proof depends on the bug.** A logic, data or state bug gets a
  test that fails first, then the fix. A purely visual one gets before/after
  screenshots and no test.
- **No sleeps or retries in tests.** Wait for the event, the promise or the
  element.
- **A rule worth keeping is a test or a lint, not a sentence.** When an agent
  keeps making the same mistake, add one.
- **A change people can see** gets one pass in a real browser: the
  `test-linker-edit-app` skill.
- **A change to how Linker-Edit drives Resolve** needs `npm run test:live` on a
  scratch project before it ships. The developer runs it; say it's needed.

## Pull requests

- Only when the developer asks. Branch from `origin/main`, open against `main`.
- **Check the PR is still open before pushing to it**; a commit pushed after
  the merge never ships:
  `[ "$(gh pr view <n> --json state -q .state)" = OPEN ] && git push`.
- Conventional titles in plain words: `fix(preview): scrubbing works on touch`.
  Fill in `.github/pull_request_template.md`.
- UI changes need before/after screenshots, as `test-linker-edit-app` says.
- Before opening one, run the review agents in `.claude/agents/` that fit the
  diff: `ui-consistency` for UI in `src/`, `test-signal` for new tests.

## Docs

- The README is written for users: plain words, what they do and see. A change
  they can see changes it. When a documented decision changes, rewrite the
  text; don't add a second account.
- A reason only one spot needs is a short comment there.
- This file is loaded into every agent's context, so `agent-context-budget.yml`
  fails a PR that grows it. Cut first; the `agent-context-increase` label
  accepts growth on purpose.

## Svelte

The Svelte MCP server (`.mcp.json`) has the Svelte 5 and SvelteKit docs: start
with `list-sections`, then `get-documentation` for every section whose use case
fits. Run `svelte-autofixer` on Svelte code you write until it reports nothing.
Offer a `playground-link` only for code not written to the project.
