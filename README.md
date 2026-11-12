# Linker-Edit

**Open-source AI for video editing and creative work.**

Linker-Edit lets AI agents work inside real creative tools. You describe what you want, such as "cut the silences", "build a rough cut from the interview", or "lay these clips out on the beat". The agent then does the work in your actual project, where you can review, tweak and finish it like any other edit.

It isn't a new editor or a black-box render service. Your footage, project and tools stay yours. The agent is one more pair of hands in the edit.

> **Status: early.** Linker-Edit is young and changing fast. Today it connects Claude and Codex to DaVinci Resolve Studio. Expect rough edges, and please report them.

## What works today

- **Chat with Claude or Codex** about your open Resolve project. They read it and edit it directly with Linker-Edit's own Resolve tools, which run through the MCP server that comes with DaVinci Resolve Studio. There's nothing extra to install.
- **Projects and threads.** Each Resolve project gets its own threads and edit history in the sidebar, whether it lives on this computer, on a network database or in Blackmagic Cloud.
- **Your existing work is safe.** The agent never modifies, renames or deletes your timelines, media, bins or projects. It builds new `AI - …` timelines next to yours instead, and Linker-Edit's tools refuse to change any other timeline.
- **No Allow clicks, no surprises.** Like T3 Code, agents start with **Full access**: changes run without asking, because the rules are built into the tools and the edit history keeps every version. Switch to **Supervised** in the composer to Allow or Deny each change yourself.
- **Transcripts.** The agent hears what's said in your footage, in most languages, on your own GPU, with a time for every word.
- **The agent sees.** It looks at frames of your footage, and at its own timelines exactly as Resolve renders them, titles and grade included, to pick shots and check its work.
- **Silence cutting.** It finds pauses straight from the audio, quickly and precisely, and builds a tightened timeline from what's left.
- **Delivery and exchange.** It renders timelines to files with any Resolve preset, format or size, and exports or imports OTIO, EDL, FCPXML and AAF.
- **Titles, color and selects.** It adds Text+ titles, applies LUTs and CDLs or copies a grade between shots, detects scene cuts, and files selects into bins with colors, flags and notes.
- **Edit history.** Every change to your Resolve project is saved to Git with what was asked and what changed, so you can ask the agent what it did, or to undo it.
- **Preview in the app.** Watch what the agent made beside the chat: a cut preview of it or any earlier version right away, or a full render with color and effects.
- **Live Resolve status** in the header: whether Resolve is reachable, and which project and timeline are open.
- **Use it from your other computers.** Over your own Tailscale, the app on a laptop or another PC drives the agents and Resolve on your edit machine.
- **Sign in with Linker, if you like.** Your [Linker](https://linker.cool) account in the sidebar; nothing needs it.
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

- **DaVinci Resolve Studio 21.1.2 or newer**, which comes with its own MCP server (`ResolveMCP`). Linker-Edit finds it in Resolve's install folder. Set Preferences → System → General → *External scripting using* to **Local**, and restart Resolve after changing it. The free edition has no MCP server.
- **Claude Code** (`claude`, logged in) and/or **Codex** (`codex`, logged in).
- **[Git](https://git-scm.com)**, for the edit history.

## Install

Download the installer for your computer from the [latest release](https://github.com/FerretMedia/linker-edit/releases/latest):

| | |
| --- | --- |
| Windows | `Linker-Edit-<version>-win-x64.exe` |
| macOS, Apple silicon | `Linker-Edit-<version>-mac-arm64.dmg` |
| macOS, Intel | `Linker-Edit-<version>-mac-x64.dmg` |
| Linux | `Linker-Edit-<version>-linux-x86_64.AppImage` |

The builds aren't signed yet, so the first launch needs one extra step:

- **Windows**: SmartScreen says it protected your PC. Click *More info* → *Run anyway*.
- **macOS**: open the app once, then System Settings → Privacy & Security → *Open Anyway*.
- **Linux**: `chmod +x` the AppImage, then run it.

Start Resolve, check that the Resolve status in the sidebar is green, then ask away. A good first prompt is "Describe the open project and its timelines".

The app downloads new versions by itself and the sidebar offers *Restart to update*. On macOS the sidebar links to the new download instead, because an unsigned app can't replace itself. Your projects, history and settings stay in `~/LinkerEdit`.

### Or run it with Node.js

With **Node.js 24+**:

```sh
npx linker-edit@latest
```

Linker-Edit opens in its own window. Closing the window stops it; `npx linker-edit@latest --browser` opens it in your browser at http://127.0.0.1:4317 instead. When a new version is out, the sidebar says so: stop Linker-Edit (Ctrl+C) and run `npx linker-edit@latest` again.

## Configuration

The defaults work for a standard install. To override them, create `~/LinkerEdit/linker-edit.config.json` (see `linker-edit.config.example.json`):

| Key | Default |
|---|---|
| `projectsDir` | `~/LinkerEdit/projects`, where new projects get their folders |
| `workspaceDir` | `~/LinkerEdit/workspace`, the working folder from before projects; it becomes your first project |
| `previewDir` | `~/LinkerEdit/previews`, where preview renders go |
| `resolveMcp` | found in Resolve's install folder: the path of Resolve's `ResolveMCP` program, for an install Linker-Edit doesn't find |
| `port` | `4317` |
| `autoCompactWindow` | unset: Claude Code decides. Tokens at which Claude compacts the chat by itself, from `100000` to `1000000` |
| `transcriptsDir` | `~/LinkerEdit/transcripts` |
| `toolsDir` | `~/LinkerEdit/tools`, the speech engine and its models |
| `tailscalePort` | `8443`, the HTTPS port remote access uses on your tailnet |
| `whisperCli` | unset: Linker-Edit downloads whisper.cpp on Windows and looks on PATH elsewhere |
| `linkerAuthServer` | `https://clerk.linker.cool`, Linker's sign-in; another one for testing against staging |
| `linkerAppUrl` | `https://app.linker.cool`, Linker's dashboard and API |
| `linkerClientId` | unset: the client Linker publishes for Linker-Edit at `app.linker.cool/oauth/linker-edit.json` |

## Development

```sh
npm install
npm run dev
```

Open http://localhost:5317, or run `npm run dev:desktop` to get the app window as well. `npm run build && npm start` serves the built app from http://127.0.0.1:4317 instead. Releases are described in [RELEASING.md](RELEASING.md).

## How approvals work

Like T3 Code, there are two modes, switched in the composer. **Full access** (the default) runs every allowed change without asking. **Supervised** shows Allow/Deny before each change. What Linker-Edit refuses, it refuses in both modes, so full access never means anything goes.

Agents work in Resolve with Linker-Edit's own tools, in its `linker` MCP server. Each tool either reads the project or changes it:

- **Reads** run straight away:
  - looking at the project: `resolve_context`, `list_timelines`, `get_timeline`, `list_media`, `get_clip_properties`, `get_transcription`, `render_options`;
  - looking at footage: `view_frames`, which moves Resolve's playhead and puts it back, and `find_silences`;
  - `export_timeline`, which only writes a file into the project folder;
  - `set_current_timeline`, which only changes which timeline Resolve shows.
- **Changes** run straight away with full access, and show Allow/Deny in Supervised mode, for Claude and Codex alike.
  - Timelines: `create_timeline`, `duplicate_timeline`, `import_timeline`.
  - Editing an AI timeline: `add_clips`, `remove_clips`, `set_clip_properties`, `add_markers`, `remove_markers`, `set_tracks`, `add_title`, `set_color`, `detect_scene_cuts`.
  - The media pool: `import_media`, `organize_media`, `transcribe_in_resolve`.
  - `render_timeline`, which puts Resolve's timeline, page and render settings back afterwards.
- **Reading the edit history** (`git log`, `git show`, `git diff`, `git status`) runs straight away. Claude's other file edits and commands ask only in Supervised mode; Codex's shell commands and file edits run sandboxed to the project's folder.

The rules are built into the tools, not only into the agent's instructions:
- they only change timelines named `AI - …`;
- none of them deletes media, bins or projects;
- none of them opens another project. You switch projects in Linker-Edit.

Claude also gets Resolve's own MCP server:
- **Its scripting API and docs**, to look things up. These run straight away.
- **Its `run_script` tool**, as an escape hatch for what Linker-Edit's tools can't do yet. With full access a script runs straight away; in Supervised mode it shows Allow/Deny with the full script.
- **Refused outright, in both modes:** scripts that open, close or switch projects or databases, delete media, bins, timelines, render jobs or render presets, or replace a clip's media.

Codex gets only the read-only docs tools, because it has no way to ask before a script. No agent gets Resolve's `run_script_unsafe` tool, or its tools for deleting LUTs or launching Resolve.

## Projects and threads

A **project** is one Resolve project with its own folder: its edit history, the agent's notes and exports, and its **threads**. A thread is one chat with an agent. Start a new one with the pen icon next to a project; rename, archive or delete one from its **⋯** menu.

Resolve has one project open at a time, so Linker-Edit follows it:

- A thread only runs when its project is the one open in Resolve. When another project is open, the composer says so, and **Open in Resolve** saves the project Resolve has open and opens this one. You can also switch in Resolve yourself.
- One thread works in Resolve at a time. The others wait until it's done.
- When Resolve has a project open that isn't in Linker-Edit yet, the sidebar offers to add it.

**Adding a project** (the **+** next to Projects):

- **Open in Resolve:** the project Resolve has open now.
- **Database:** any project in the database Resolve has open, local or network (PostgreSQL). To add one from another database, switch to that database in Resolve's Project Manager first.
- **Cloud:** a Blackmagic Cloud project, by its name and a folder for its media on this computer. Resolve can't list cloud projects to other apps, so if it's open in Resolve, use **Open in Resolve** instead.
- **Folder:** a project folder from another computer, such as a teammate's edit history you cloned.

Folder fields list the folders as you type, like T3 Code's **Add project**. Use ↑/↓ to choose a folder, and Enter or Tab to open it. Enter with no folder chosen picks the path you typed. Project folders are marked **Linker-Edit project**. The folders listed are on the computer running Linker-Edit, even when you use it from another one. In the desktop app on that computer, **Browse…** also opens the system's folder dialog.

A project with no Resolve project yet links itself to the one open in Resolve with its first message. To link it to another one later, open that one in Resolve and use **Project settings**. Projects stay linked when you rename them in Resolve.

Each project folder has a `linker-edit.json` that says which Resolve project it belongs to. It's committed with the edit history, so a teammate who clones it gets the link. What differs per computer, such as the database name and the cloud media folder, stays in `.linker-edit/local.json`, which Git ignores. Removing a project from Linker-Edit keeps its folder.

## Edit history

Each project folder is a Git repository. Linker-Edit saves the open Resolve project in it:

- **After every turn in which the agent changed Resolve.** The commit is authored by Claude or Codex and holds your prompt, the Resolve actions and a summary such as `"AI - Rough cut": 3 clips removed, length 4:02 → 3:41`.
- **Before every turn, if you changed something in Resolve yourself.** The agent can tell your edits from its own. In a Blackmagic Cloud project, these saves are labelled as changes made outside Linker-Edit, since a collaborator may have made them.

Each timeline is a folder under `resolve/<project>/` with `cuts.json`, `audio.json`, `markers.json` and `metadata.json`, one clip per line, so `git log -p` reads like an edit decision list. Media is never stored.

**History** in the top right lists the project's saves, newest first, with what changed per timeline; saves made in the open thread are marked. **Restore** on a save writes the undo request into the composer for you to send.

Ask "what did you change?" or "undo the last change". The agent reads this history and rebuilds the earlier version as a new `AI - … (restored …)` timeline; it never edits or deletes a timeline to undo. Color grades and effects aren't saved yet, so a rebuilt version has the cuts, clip settings and markers, not the grade.

You can share the history like any Git repository (`git remote add`, `git push`), but merging two people's histories back into Resolve isn't supported yet.

## Long chats

Everything said and every Resolve result stays in the agent's context for the rest of a chat, and each turn rereads it. Linker-Edit keeps that in check:

- **Context meter.** With Claude, the ring next to the send button shows how full the context is. Click it for the token count and **Compact now**, which replaces the chat so far with a summary. You can keep chatting afterwards.
- **Compact on return.** Claude keeps a recent chat cached for about an hour on a subscription (5 minutes with an API key). If you come back to a chat over 200,000 tokens after that, a **Compact** chip shows next to the send button: sending then summarizes the chat first, so all of it isn't reread at full price. Click the chip to switch to **Full** and send with the whole chat.
- **Long Resolve results are shortened.** A Resolve result over about 5,000 tokens, such as every item on a long timeline, has its lists cut down before the agent reads it. The agent is told to ask for a narrower part if it needs more.
- **Threads survive restarts.** Each thread is kept in `.linker-edit/threads/` in its project folder, which Git ignores, and the agent continues the same conversation. If it can't (say Claude's session was deleted), it starts a new one with a summary of the chat.
- **Switching agent keeps the chat.** The new agent gets a short summary of it with your next message: what was asked, what was said, and what changed in Resolve. It doesn't get the other agent's full context.

Codex has no context meter or compaction here: its SDK doesn't report how full the context is or run `/compact`.

## Preview

**Preview** in the top right plays a timeline beside the chat, in two ways:

- **Cut preview** starts right away. It plays your media straight from disk, following the cuts, with zoom, position and opacity. Color, effects, transitions and titles only show in a render, and formats browsers can't decode (ProRes, BRAW and similar) show as a placeholder. It also plays earlier versions: **Play** on a timeline in the history opens it as it was in that save.
- **Full render** has Resolve render the exact picture as a small MP4, and the player switches to it at the same spot. While it renders, Resolve switches to that timeline and the Deliver page, then puts your timeline, page and render settings back. The agent waits until the render is done, and you can stop it. Only the timeline as it is in Resolve now can be rendered.

Keys: Space play or pause, ← → one frame (Shift: one second), ↑ ↓ previous or next cut, F full screen, M mute.

## Transcripts

Ask for anything that depends on what's said, such as "transcribe the interview clips" or "cut the bits where I mention pricing". The agent transcribes the clips it needs with [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (`large-v3-turbo`) on this machine. Nothing is uploaded, and it costs nothing.

- **First use downloads the speech engine** into `~/LinkerEdit/tools`: about 1.5 GB on Windows with an NVIDIA GPU, or 0.9 GB without one, where it runs on the CPU and is much slower. The sidebar shows the progress. On macOS and Linux, install whisper.cpp first (`brew install whisper-cpp` on a Mac) so `whisper-cli` is on your PATH, or point `whisperCli` in the config at it.
- **It's fast.** On an RTX 2060, a 3-minute clip takes about 9 seconds, model loading included.
- **Each transcript is kept** in `~/LinkerEdit/transcripts`, as text and as JSON with per-word times, until the clip's file changes. Asking again is instant.
- **Languages are detected per clip** by sampling it from start to end, so a clip that opens in English and goes on in Dutch is transcribed as Dutch, with the English parts kept in English. If you know the language, say so in your prompt.
- **Transcripts in Resolve itself:** ask for it, and the agent uses Resolve's own transcription instead. It stores the transcript on the clip, where you can edit by text. It never adds subtitles to a timeline unless you ask.

## Remote access

Chat from another computer, such as a laptop on the couch or a PC at the office, while Resolve, your footage and the agents stay on your edit machine. It goes over your own [Tailscale](https://tailscale.com) network, so nothing is opened to the internet.

1. Install Tailscale on both computers and log in to the same account.
2. On the edit machine, open the computer menu at the top of the sidebar, choose **Remote access…**, then **Turn on**.
3. On the other computer, run Linker-Edit too, open the same computer menu and pick the edit machine. Everything you send now goes to the edit machine; pick **this computer** to go back.

Only your own computers show up in that menu, and only once remote access is on and Linker-Edit is running on them. The address in the dialog, like `https://studio.tail1234.ts.net:8443`, also works in a browser on your devices, such as a tablet.

- **Only your devices get in.** Linker-Edit lets in devices logged in to the Tailscale account that owns the edit machine. Other people on a shared tailnet and tagged devices are turned away.
- **The first time, Tailscale may ask you to allow it.** If Tailscale Serve or HTTPS certificates are off for your tailnet, the dialog shows Tailscale's link to turn them on. Then click **Turn on** again.
- **It stays on** after Linker-Edit or the computer restarts, until you turn it off in the same dialog. While Linker-Edit isn't running, the address shows an error.
- **It uses port 8443**, so it doesn't take over an address you already serve on 443, such as T3 Code's. If 8443 is taken too, set `tailscalePort` in the config.

## Sign in with Linker

Optional. Press **Sign in with Linker** at the bottom of the sidebar and sign in with your [Linker](https://linker.cool) account in your browser. The sidebar then shows who you are and your plan. Nothing else changes yet: this is the start of working on your Linker projects from Linker-Edit (see [docs/linker-plan.md](docs/linker-plan.md)).

- **Sign in on the edit machine itself.** Linker sends your browser back to Linker-Edit on `127.0.0.1`, so it works on the computer Linker-Edit runs on, on the default port 4317.
- **Linker-Edit asks for your name and email,** and to stay signed in. It keeps the sign-in in `~/LinkerEdit/linker-account.json`, readable only by you. **Sign out** in the account's menu removes it and tells Linker to forget it.

## Testing

- `npm test`: unit tests. Free and offline. With Python installed, they also run every Resolve script (`server/resolve/scripts`) against a stand-in Resolve, and the frame and silence tools against media made with ffmpeg.
- `npm run test:e2e`: drives the built app in a browser (`npx playwright install chromium` once). Claude is replaced by a recorded turn, so it costs nothing.
- `LINKER_EDIT_REPLAY=server/providers/fixtures/claude-turn.jsonl npm run dev`: the same replay, for UI work without spending tokens.
- `npm run record-fixture -- <claude|codex> <out.jsonl> "<prompt>"`: records a real turn on the cheapest models, for new provider tests.
- `npm run test:live -- --project "<scratch project>"`: runs real editing prompts against Resolve with approvals off, then checks that existing timelines and media are untouched. It then compacts the chat and continues it in a new session, as after a restart. It costs tokens; run it before releases and after Resolve updates. `--dry-run` only checks that it can see the project.

## Contributing

Linker-Edit is built in the open, and it's meant to stay small and obvious. The most useful contributions right now are:

- **Bug reports from real edits:** what you asked for, what the agent did, and what went wrong.
- **Workflow ideas:** editing tasks you'd hand to an assistant if you could.
- **Fixes**, especially to the Resolve integration and the guardrails.

For anything larger, please open an issue before writing the code, so we can agree on the approach.

## Credits

MIT licensed. Speech to text by [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (MIT) with OpenAI's Whisper model, and audio decoding by [FFmpeg](https://ffmpeg.org) through `ffmpeg-static` (GPL), both run as separate programs. Linker-Edit drives Resolve through the MCP server Blackmagic Design ships with DaVinci Resolve Studio; it starts your installed copy and doesn't distribute it. The edit history format follows [Vit](https://github.com/LucasHJin/vit). The interface is modelled on [T3 Code](https://github.com/pingdotgg/t3code); its theme tokens and provider icons are adapted from T3 Code, MIT licensed, Copyright (c) 2026 T3 Tools Inc. Built with SvelteKit. DaVinci Resolve is a trademark of Blackmagic Design; Linker-Edit is not affiliated with Blackmagic Design.
