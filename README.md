# Linker-Edit

**Open-source AI for video editing and creative work.**

Linker-Edit lets AI agents work inside real creative tools. You describe what you want, such as "cut the silences", "build a rough cut from the interview", or "lay these clips out on the beat". The agent then does the work in your actual project, where you can review, tweak and finish it like any other edit.

It isn't a new editor or a black-box render service. Your footage, project and tools stay yours. The agent is one more pair of hands in the edit.

> **Status: early.** Linker-Edit is young and changing fast. Today it connects Claude and Codex to DaVinci Resolve Studio. Expect rough edges, and please report them.

## What works today

- **Chat with Claude or Codex** about your open Resolve project. They read it and edit it directly through [davinci-resolve-mcp](https://github.com/samuelgursky/davinci-resolve-mcp).
- **Your existing work is safe.** The agent never modifies, renames or deletes your timelines, media, bins or projects. It builds new `AI/…` timelines next to yours instead.
- **You stay in control.** With "Ask before changes" on, every change waits for your Allow or Deny, while reads run straight away.
- **Live Resolve status** in the header: whether Resolve is reachable, and which project and timeline are open.
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
- A **davinci-resolve-mcp** checkout with a Python 3.10–3.12 virtualenv, next to this folder by default:

  ```sh
  git clone https://github.com/samuelgursky/davinci-resolve-mcp ../davinci-resolve-mcp
  cd ../davinci-resolve-mcp && uv venv --python 3.12 && uv pip install -r requirements.txt
  ```

## Run

```sh
npm install
npm run dev
```

Open http://localhost:5317 and check that the status pill in the header is green, then ask away. A good first prompt is "Describe the open project and its timelines".

`npm run build && npm start` serves the built app from http://127.0.0.1:4317 instead.

## Configuration

The defaults work for a standard install. To override them, copy `linker-edit.config.example.json` to `linker-edit.config.json`:

| Key | Default |
|---|---|
| `workspaceDir` | `~/LinkerEdit/workspace`, the agent's working folder for notes and exports |
| `resolveMcpDir` | `../davinci-resolve-mcp` |
| `python` | the checkout's `.venv` Python |
| `env` | Resolve scripting paths for your OS |
| `port` | `4317` |

## How approvals work

- **Claude, with "Ask before changes" on:**
  - Resolve actions that only read (get, list, summaries, reports, plans) run immediately.
  - Any other Resolve action, and any file edit, shows Allow/Deny.
- **Claude, with it off:** everything runs.
- **Codex:** runs without per-call approvals, because its SDK has no approval hook. Its file writes are sandboxed to the workspace folder.

## Testing

- `npm test`: unit tests. Free and offline. They include a list of every davinci-resolve-mcp action that runs without approval (`server/__snapshots__/resolve-read-actions.txt`). After updating the MCP, review that diff before accepting it with `npx vitest -u`.
- `npm run test:e2e`: drives the built app in a browser (`npx playwright install chromium` once). Claude is replaced by a recorded turn, so it costs nothing.
- `LINKER_EDIT_REPLAY=server/providers/fixtures/claude-turn.jsonl npm run dev`: the same replay, for UI work without spending tokens.
- `npm run record-fixture -- <claude|codex> <out.jsonl> "<prompt>"`: records a real turn on the cheapest models, for new provider tests.
- `npm run test:live -- --project "<scratch project>"`: runs real editing prompts against Resolve with approvals off, then checks that existing timelines and media are untouched. It costs tokens; run it before releases and after MCP updates. `--dry-run` only checks that it can see the project.

## Contributing

Linker-Edit is built in the open, and it's meant to stay small and obvious. The most useful contributions right now are:

- **Bug reports from real edits:** what you asked for, what the agent did, and what went wrong.
- **Workflow ideas:** editing tasks you'd hand to an assistant if you could.
- **Fixes**, especially to the Resolve integration and the guardrails.

For anything larger, please open an issue before writing the code, so we can agree on the approach.

## Credits

MIT licensed. Resolve integration by [davinci-resolve-mcp](https://github.com/samuelgursky/davinci-resolve-mcp). The interface is modelled on [T3 Code](https://github.com/pingdotgg/t3code); its theme tokens and provider icons are adapted from T3 Code, MIT licensed, Copyright (c) 2026 T3 Tools Inc. Built with SvelteKit. DaVinci Resolve is a trademark of Blackmagic Design; Linker-Edit is not affiliated with Blackmagic Design.
