# Linker-Edit

**Open-source AI for video editing and creative work.**

Linker-Edit lets AI agents work inside real creative tools. You describe what you want, such as "cut the silences", "build a rough cut from the interview", or "lay these clips out on the beat". The agent then does the work in your actual project, where you can review, tweak and finish it like any other edit.

It isn't a new editor or a black-box render service. Your footage, project and tools stay yours. The agent is one more pair of hands in the edit.

> **Status: early.** Linker-Edit is young and changing fast. Today it connects Claude and Codex to DaVinci Resolve Studio. Expect rough edges, and please report them.

## What works today

- **Chat with Claude or Codex** about your open Resolve project. They read it and edit it directly through [davinci-resolve-mcp](https://github.com/samuelgursky/davinci-resolve-mcp).
- **Your existing work is safe.** The agent never modifies, renames or deletes your timelines, media, bins or projects. It builds new `AI - …` timelines next to yours instead.
- **You stay in control.** With "Ask before changes" on, every change waits for your Allow or Deny, while reads run straight away.
- **Edit history.** Every change to your Resolve project is saved to Git with what was asked and what changed, so you can ask the agent what it did, or to undo it.
- **Preview in the app.** Watch what the agent made beside the chat: a cut preview of it or any earlier version right away, or a full render with color and effects.
- **Live Resolve status** in the header: whether Resolve is reachable, and which project and timeline are open.
- **Long chats stay affordable.** A context meter, compaction, chats that survive restarts, and switching between Claude and Codex mid-chat.
- **Uses the subscriptions you already have.** Linker-Edit drives your logged-in `claude` and `codex` CLIs, with no extra API keys.

## Where it's going

The goal is a single open place where AI does the tedious parts of video and creative work, so people can spend their time on the creative decisions. Directions we're exploring, in roughly this order:

- **Everyday editing chores:** silence and filler removal, rough cuts from transcripts, selects, string-outs, b-roll placement, music cut to the beat.
- **Understanding footage:** transcripts, shot detection and searchable media, so you can ask for "every shot of the beach at sunset".
- **Repeatable workflows:** save a recipe once ("podcast episode to three vertical shorts") and run it on new footage.
- **More tools:** Resolve comes first; other editors and creative apps follow where they can be scripted.

Priorities come from real use. If something here matters to you, or is missing, open an issue.

## Requirements

- **DaVinci Resolve Studio** with Preferences → System → General → *External scripting using* set to **Local**. Restart Resolve after changing it. The free edition blocks external scripting.
- **Node.js 24+**
- **Claude Code** (`claude`, logged in) and/or **Codex** (`codex`, logged in).
- **[Git](https://git-scm.com)** and **[uv](https://docs.astral.sh/uv/)**, which Linker-Edit uses to install [davinci-resolve-mcp](https://github.com/samuelgursky/davinci-resolve-mcp) and Python for it.

## Run

```sh
npx linker-edit@latest
```

The first run installs davinci-resolve-mcp into `~/LinkerEdit/davinci-resolve-mcp`. Then Linker-Edit opens http://127.0.0.1:4317 in your browser. Check that the Resolve status in the sidebar is green, then ask away. A good first prompt is "Describe the open project and its timelines".

When a new version is out, the sidebar says so. Stop Linker-Edit (Ctrl+C) and run `npx linker-edit@latest` again. Your workspace, history and settings stay in `~/LinkerEdit`.

## Configuration

The defaults work for a standard install. To override them, create `~/LinkerEdit/linker-edit.config.json` (see `linker-edit.config.example.json`):

| Key | Default |
|---|---|
| `workspaceDir` | `~/LinkerEdit/workspace`, the agent's working folder for notes and exports |
| `previewDir` | `~/LinkerEdit/previews`, where preview renders go |
| `resolveMcpDir` | `~/LinkerEdit/davinci-resolve-mcp` |
| `python` | the checkout's `.venv` Python |
| `env` | Resolve scripting paths for your OS |
| `port` | `4317` |
| `autoCompactWindow` | unset: Claude Code decides. Tokens at which Claude compacts the chat by itself, from `100000` to `1000000` |

## Development

```sh
npm install
npm run setup   # installs davinci-resolve-mcp, once
npm run dev
```

Open http://localhost:5317. `npm run build && npm start` serves the built app from http://127.0.0.1:4317 instead. Releases are described in [RELEASING.md](RELEASING.md).

## How approvals work

- **Claude, with "Ask before changes" on:**
  - Resolve actions that only read (get, list, summaries, reports, plans) run immediately.
  - Reading the edit history (`git log`, `git show`, `git diff`, `git status`) runs immediately.
  - Any other Resolve action, and any file edit, shows Allow/Deny.
- **Claude, with it off:** everything runs.
- **Codex:** runs without per-call approvals, because its SDK has no approval hook. Its file writes are sandboxed to the workspace folder.

## Edit history

The workspace folder is a Git repository. Linker-Edit saves the open Resolve project in it:

- **After every turn in which the agent changed Resolve.** The commit is authored by Claude or Codex and holds your prompt, the Resolve actions and a summary such as `"AI - Rough cut": 3 clips removed, length 4:02 → 3:41`.
- **Before every turn, if you changed something in Resolve yourself.** The agent can tell your edits from its own.

Each timeline is a folder under `resolve/<project>/` with `cuts.json`, `audio.json`, `markers.json` and `metadata.json`, one clip per line, so `git log -p` reads like an edit decision list. Media is never stored.

**History** in the top right lists every save, newest first, with what changed per timeline. **Restore** on a save writes the undo request into the composer for you to send.

Ask "what did you change?" or "undo the last change". The agent reads this history and rebuilds the earlier version as a new `AI - … (restored …)` timeline; it never edits or deletes a timeline to undo. Color grades and effects aren't saved yet, so a rebuilt version has the cuts, clip settings and markers, not the grade.

You can share the history like any Git repository (`git remote add`, `git push`), but merging two people's histories back into Resolve isn't supported yet.

## Long chats

Everything said and every Resolve result stays in the agent's context for the rest of a chat, and each turn rereads it. Linker-Edit keeps that in check:

- **Context meter.** With Claude, the ring next to the send button shows how full the context is. Click it for the token count and **Compact now**, which replaces the chat so far with a summary. You can keep chatting afterwards.
- **Compact on return.** Claude keeps a recent chat cached for about an hour on a subscription (5 minutes with an API key). If you come back to a chat over 200,000 tokens after that, a **Compact** chip shows next to the send button: sending then summarizes the chat first, so all of it isn't reread at full price. Click the chip to switch to **Full** and send with the whole chat.
- **Long Resolve results are shortened.** With Claude, a Resolve result over about 5,000 tokens, such as every item on a long timeline, has its lists cut down before the agent reads it. The agent is told to ask for a narrower part if it needs more.
- **Chats survive restarts.** The open chat is kept in `.linker-edit/` in the working folder, which Git ignores, and the agent continues the same conversation. If it can't (say Claude's session was deleted), it starts a new one with a summary of the chat.
- **Switching agent keeps the chat.** The new agent gets a short summary of it with your next message: what was asked, what was said, and what changed in Resolve. It doesn't get the other agent's full context.

Codex has no context meter or compaction here: its SDK doesn't report how full the context is or run `/compact`. Its results aren't shortened either.

## Preview

**Preview** in the top right plays a timeline beside the chat, in two ways:

- **Cut preview** starts right away. It plays your media straight from disk, following the cuts, with zoom, position and opacity. Color, effects, transitions and titles only show in a render, and formats browsers can't decode (ProRes, BRAW and similar) show as a placeholder. It also plays earlier versions: **Play** on a timeline in the history opens it as it was in that save.
- **Full render** has Resolve render the exact picture as a small MP4, and the player switches to it at the same spot. While it renders, Resolve switches to that timeline and the Deliver page, then puts your timeline, page and render settings back. The agent waits until the render is done, and you can stop it. Only the timeline as it is in Resolve now can be rendered.

Keys: Space play or pause, ← → one frame (Shift: one second), ↑ ↓ previous or next cut, F full screen, M mute.

## Testing

- `npm test`: unit tests. Free and offline. They include a list of every davinci-resolve-mcp action that runs without approval (`server/__snapshots__/resolve-read-actions.txt`). After updating the MCP, review that diff before accepting it with `npx vitest -u`.
- `npm run test:e2e`: drives the built app in a browser (`npx playwright install chromium` once). Claude is replaced by a recorded turn, so it costs nothing.
- `LINKER_EDIT_REPLAY=server/providers/fixtures/claude-turn.jsonl npm run dev`: the same replay, for UI work without spending tokens.
- `npm run record-fixture -- <claude|codex> <out.jsonl> "<prompt>"`: records a real turn on the cheapest models, for new provider tests.
- `npm run test:live -- --project "<scratch project>"`: runs real editing prompts against Resolve with approvals off, then checks that existing timelines and media are untouched. It then compacts the chat and continues it in a new session, as after a restart. It costs tokens; run it before releases and after MCP updates. `--dry-run` only checks that it can see the project.

## Contributing

Linker-Edit is built in the open, and it's meant to stay small and obvious. The most useful contributions right now are:

- **Bug reports from real edits:** what you asked for, what the agent did, and what went wrong.
- **Workflow ideas:** editing tasks you'd hand to an assistant if you could.
- **Fixes**, especially to the Resolve integration and the guardrails.

For anything larger, please open an issue before writing the code, so we can agree on the approach.

## Credits

MIT licensed. Resolve integration by [davinci-resolve-mcp](https://github.com/samuelgursky/davinci-resolve-mcp). The edit history format follows [Vit](https://github.com/LucasHJin/vit). The interface is modelled on [T3 Code](https://github.com/pingdotgg/t3code); its theme tokens and provider icons are adapted from T3 Code, MIT licensed, Copyright (c) 2026 T3 Tools Inc. Built with SvelteKit. DaVinci Resolve is a trademark of Blackmagic Design; Linker-Edit is not affiliated with Blackmagic Design.
