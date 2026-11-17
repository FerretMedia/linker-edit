---
name: test-linker-edit-app
description: Run Linker-Edit from a worktree and check a change in a real browser, with free replayed agent turns and data of its own, without touching the developer's Resolve project or ~/LinkerEdit. Use to verify a visible change, take before/after screenshots for a PR, or reproduce a UI bug.
---

# Test Linker-Edit in a browser

Paths are from the repository root; the app is in `apps/desktop`. Read the root
`AGENTS.md`'s "five ways to hurt yourself" first: the developer's Resolve,
`~/LinkerEdit`, and their dev server on 5317 and 4317 are off limits.

## Start the app

`npm run dev` has fixed ports (Vite on 5317, proxying to 4317), so serve a
build on a port of your own instead, as the e2e tests do. Make a folder outside
the repository, say `<tmp>/le-agent`, with two files:

`linker-edit.config.json`, which keeps every folder in there and leaves Resolve
out (an empty `resolveMcp` reads as "no Resolve on this computer", like CI):

```json
{
  "port": 4380,
  "projectsDir": "<tmp>/le-agent/projects",
  "workspaceDir": "<tmp>/le-agent/workspace",
  "previewDir": "<tmp>/le-agent/previews",
  "transcriptsDir": "<tmp>/le-agent/transcripts",
  "toolsDir": "<tmp>/le-agent/tools",
  "resolveMcp": ""
}
```

`settings.json`, so the server doesn't download the 1.5 GB speech engine:

```json
{ "resolveHere": false }
```

Then build and start the server from `apps/desktop` in the background, keeping
its PID:

```sh
cd apps/desktop
npm run build
LINKER_EDIT_CONFIG=<tmp>/le-agent/linker-edit.config.json \
LINKER_EDIT_REPLAY=server/providers/fixtures/claude-turn.jsonl \
node server/index.ts
```

Open `http://127.0.0.1:4380`. With `LINKER_EDIT_REPLAY`, a sent message plays a
recorded Claude turn: free, and it never reaches Resolve. Rebuild after each
change to `src/`; restart the server after a change to `server/`.

To work with real Resolve data or a real agent, ask the developer first and use
a scratch Resolve project they name.

## Use a browser

**T3 Code's Browser panel first.** `preview_status`; if no tab can be
automated, `preview_open` with the `incognito` profile. Then `preview_navigate`,
`preview_snapshot`, the interaction tools, and `preview_resize` to check narrow
widths. A snapshot times out while the panel isn't on screen; ask the developer
to show it. If it reports "No preview automation host", it won't come back this
session.

**Then Playwright.** `@playwright/test` is a dev dependency
(`npx playwright install chromium` once). Write the throwaway script outside
the repository; for a lasting check, add a case to `apps/desktop/e2e/` instead.

## Finish

Capture what proves the change (before and after, light and dark where it
matters) for the PR. Keep the server up while the developer is still looking;
when done, stop only your own, by its PID, and delete `<tmp>/le-agent`.

Put the screenshots in the PR body with the Markdown this prints:

```sh
node .agents/skills/test-linker-edit-app/scripts/pr-screenshots.mjs <branch-name> before.png after.png
```

It pushes them to a branch of their own, `pr-assets/<branch-name>`, so they
are never committed to the PR.
