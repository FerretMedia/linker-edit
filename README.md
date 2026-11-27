# Linker-Edit

**Open-source AI for video editing and creative work.**

Linker-Edit lets AI agents work inside real creative tools. You describe what you want, such as "cut the silences", "build a rough cut from the interview", or "lay these clips out on the beat". The agent then does the work in your actual project, where you can review, tweak and finish it like any other edit.

It isn't a new editor or a black-box render service. Your footage, project and tools stay yours. The agent is one more pair of hands in the edit.

> **Status: early.** Linker-Edit is young and changing fast. Today it connects Claude and Codex to DaVinci Resolve Studio. Expect rough edges, and please report them.

## What works today

- **Chat with Claude or Codex** about your open Resolve project. They read it and edit it directly with Linker-Edit's own Resolve tools, which run through the MCP server that comes with DaVinci Resolve Studio. There's nothing extra to install.
- **Projects and threads.** Each Resolve project gets its own threads and edit history in the sidebar, whether it lives on this computer, on a network database or in Blackmagic Cloud.
- **Your existing work is safe.** The agent never modifies, renames or deletes your timelines, media, bins or projects. It builds new `AI - …` timelines next to yours instead, and Linker-Edit's tools refuse to change any other timeline.
- **No Allow clicks, no surprises.** Like T3 Code, agents start with **Full access**: changes run without asking. The project is backed up before each turn's first change, agents work on their own `AI - …` copies, and the edit history keeps every version. Switch to **Supervised** in the composer to Allow or Deny each change yourself.
- **You decide what becomes the real edit.** When you're happy with an AI version of a timeline, click **Use as main timeline** in the preview. It takes your timeline's name, and your old one stays next to it as `<name> (before AI …)`.
- **Transcripts.** The agent hears what's said in your footage, in most languages, on your own GPU, with a time for every word.
- **The agent sees.** It looks at frames of your footage, and at its own timelines exactly as Resolve renders them, titles and grade included, to pick shots and check its work.
- **Your library.** Point Linker-Edit at the folders where you keep music, sound effects, logos and LUTs. The agent looks there first and brings files into the project, and you play, star and tag them on the library page.
- **Silence cutting.** It finds pauses straight from the audio, quickly and precisely, and builds a tightened timeline from what's left.
- **Delivery and exchange.** It renders timelines to files with any Resolve preset, format or size, and exports or imports OTIO, EDL, FCPXML and AAF.
- **Titles, color and selects.** It adds Text+ titles, applies LUTs and CDLs or copies a grade between shots, detects scene cuts, and files selects into bins with colors, flags and notes.
- **Edit history.** Every change to your Resolve project is saved to Git with what was asked and what changed, so you can ask the agent what it did, or to undo it.
- **Preview in the app.** Watch what the agent made beside the chat: a cut preview of it or any earlier version right away, or a full render with color and effects.
- **Live Resolve status** in the header: whether Resolve is reachable, and which project and timeline are open.
- **All your computers in one sidebar.** Like T3 Code, each project lives on one computer, and the sidebar lists the projects of all your computers together, so two threads can work on two edit machines at once. Over Linker Connect or your own Tailscale, a laptop drives the agents and Resolve on your edit machines. On a computer that only does that, click **Resolve won't be on this computer** under *Setup needed*, or turn off *Resolve is on this computer* in Settings: the setup and the speech engine are left out there.
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
- **Claude Code** (`claude`, logged in) and/or **Codex** (`codex`, logged in). Linker-Edit uses your sign-in, but not your own setup for coding: chats leave out your skills, your global `CLAUDE.md`, connectors such as Gmail, and for Claude your MCP servers. Codex is told to ignore your global `AGENTS.md`, and still loads the MCP servers in its `config.toml`.
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

The app looks for a new version a few seconds after it starts and then every hour; click the version next to the logo to look now. It downloads a new version by itself, and the sidebar shows what's new and offers *Restart to update*. If an agent is still working in Resolve, it restarts once the turn is done instead of stopping it halfway through an edit. Quitting the app installs a downloaded update too. On macOS the sidebar links to the new download instead, because an unsigned app can't replace itself. Your projects, history and settings stay in `~/LinkerEdit`.

### Or run it with Node.js

With **Node.js 24+**:

```sh
npx linker-edit@latest
```

Linker-Edit opens in its own window. Closing the window stops it; `npx linker-edit@latest --browser` opens it in your browser at http://127.0.0.1:4317 instead. When a new version is out, the sidebar says so: stop Linker-Edit (Ctrl+C) and run `npx linker-edit@latest` again.

## Configuration

**Settings** (the gear at the bottom of the sidebar, or Ctrl+, / ⌘,) has what you change day to day: approvals, the agent new threads start with, the theme, how the sidebar groups and settles threads, Resolve on this computer, the speech engine, remote access, your Linker account and updates. It also shows where Linker-Edit keeps its files. Each computer has its own settings: with more than one, choose the computer at the top of the settings list. It starts on the computer of the thread you have open.

The defaults work for a standard install. To change where things go, create `~/LinkerEdit/linker-edit.config.json` (see `linker-edit.config.example.json`):

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
| `whisperCli` | unset: Linker-Edit downloads whisper.cpp on Windows; on Linux and macOS it looks on PATH, then downloads it |
| `linkerAuthServer` | `https://clerk.linker.cool`, Linker's sign-in; another one for testing against staging |
| `linkerAppUrl` | `https://app.linker.cool`, Linker's dashboard and API |
| `linkerClientId` | unset: Linker-Edit's own OAuth app in Linker's Clerk (`okxg8tSdwQEu1rbv`). With another `linkerAuthServer`, the client Linker publishes at `app.linker.cool/oauth/linker-edit.json` |
| `cloudflared` | unset: Linker Connect uses a `cloudflared` on PATH, or downloads its pinned release into `toolsDir` |

## Development

```sh
npm install
npm run dev
```

Open http://localhost:5317, or run `npm run dev:desktop` to get the app window as well. `npm run build && npm start` serves the built app from http://127.0.0.1:4317 instead. Releases are described in [RELEASING.md](RELEASING.md).

### Repo layout

The repository is an npm workspaces monorepo. Run `npm install` and the `npm run` commands from its root; they forward to the app, except `npm test` and `npm run typecheck`, which run in every workspace.

- `apps/desktop`: the app, published to npm as `linker-edit`: the Node server (`server/`), the UI (`src/`), the Electron window (`desktop/`), their tests and build scripts. Its commands run in this folder, so a relative path you give them, like `LINKER_EDIT_REPLAY` below, is from here.
- `apps/ipad`: the native iPad app (SwiftUI), a remote for the desktop app on your edit computer. It isn't released yet; see [its README](apps/ipad/README.md). It builds only on macOS, so pull requests that change it are built and tested on GitHub's Mac runners.
- `packages/`: libraries the apps share, each a workspace with its own `package.json`.
  - `packages/drp` (`@linker-edit/drp`): reads and patches DaVinci Resolve `.drp` project and `.drt` timeline files without Resolve. See [its README](packages/drp/README.md).
- `docs/`: plans.

## How approvals work

Like T3 Code, there are two modes, switched in the composer. **Full access** (the default) runs every allowed change without asking: Claude runs with `bypassPermissions`, and Codex without a sandbox. **Supervised** shows Allow/Deny before each change, and keeps Codex sandboxed to the project's folder. The mode you pick is kept across restarts, in `~/LinkerEdit/settings.json`. What Linker-Edit refuses, it refuses in both modes, so full access never means anything goes.

Three things keep full access safe:
- **Backups.** Before the first change to Resolve in each turn, Linker-Edit saves the project and exports it as a `.drp` file into `backups/` in the project's folder (the newest 30 are kept). If that fails, the change doesn't run. Restore one with **File → Import Project** in Resolve.
- **AI copies.** Agents only change `AI - …` timelines: a copy of yours (`duplicate_timeline`), or a new one that says which of yours it replaces (`create_timeline` with `replaces`).
- **You apply.** Only you make an AI timeline the main one, with **Use as main timeline** in the preview panel (it asks once more). Nothing is overwritten: your timeline is renamed `<name> (before AI <date>)` and the AI one takes its name. Agents have no tool for this.

Agents work in Resolve with Linker-Edit's own tools, in its `linker` MCP server. Each tool either reads the project or changes it:

- **Reads** run straight away:
  - `start_resolve`, which starts Resolve when it isn't running and opens the thread's own project; see [Starting Resolve](#projects-and-threads);
  - `list_projects`, `visit_project` and `end_visit`, to look at your other projects;
  - looking at the project: `resolve_context`, `list_timelines`, `get_timeline`, `list_media`, `get_clip_properties`, `get_transcription`, `render_options`;
  - looking at footage: `view_frames`, which moves Resolve's playhead and puts it back, and `find_silences`;
  - `export_timeline`, which only writes a file into the project folder;
  - `set_current_timeline`, which only changes which timeline Resolve shows;
  - `library_search`, which only looks in your library;
  - `read_skill`, which only reads [skills](#skills);
  - `study_media` and `compare_cut`, which only measure and transcribe video files.
- **Changes** run straight away with full access, and show Allow/Deny in Supervised mode, for Claude and Codex alike.
  - Timelines: `create_timeline`, `duplicate_timeline`, `import_timeline`, `restore_timeline`.
  - Editing an AI timeline: `add_clips`, `remove_clips`, `set_clip_properties`, `add_markers`, `remove_markers`, `set_tracks`, `add_title`, `set_color`, `detect_scene_cuts`.
  - The media pool: `import_media`, `library_import`, `organize_media`, `transcribe_in_resolve`.
  - `render_timeline`, which puts Resolve's timeline, page and render settings back afterwards.
- **Changes to a brief always ask**, with full access too: new rules for a brand, sub-brand or series (`save_brief`) and a lesson from your feedback (`add_lesson`) steer every later edit, so you accept or decline each. The agent's own notes on an example video (`save_example`) don't ask. See [Teaching it a brand](#teaching-it-a-brand).
- **Reading the edit history** (`git log`, `git show`, `git diff`, `git status`) runs straight away. Claude's other file edits and commands ask only in Supervised mode; Codex's shell commands and file edits run without a sandbox with full access, and sandboxed to the project's folder in Supervised mode.

The rules are built into the tools, not only into the agent's instructions:
- they only change timelines named `AI - …`;
- none of them deletes media, bins or projects;
- an agent only ever changes its own thread's project. Others it can visit, read only; see [Projects and threads](#projects-and-threads).

Claude also gets Resolve's own MCP server:
- **Its scripting API and docs**, to look things up. These run straight away.
- **Its `run_script` tool**, as an escape hatch for what Linker-Edit's tools can't do yet. The project is backed up first. With full access a script runs straight away; in Supervised mode it shows Allow/Deny with the full script.
- **Refused outright, in both modes:** scripts that open, close or switch projects or databases, delete media, bins, timelines, render jobs or render presets, or replace a clip's media.

These refusals run in a hook that Claude's `bypassPermissions` doesn't skip. Codex gets only the read-only docs tools, because it has no hook to check a script or back the project up first. No agent gets Resolve's `run_script_unsafe` tool, its `launch_resolve` tool (`start_resolve` replaces it), or its tools for deleting LUTs.

## Projects and threads

A **project** is one Resolve project with its own folder: its edit history, `PROJECT.md` (what this video is, kept by the agent), exports, and its **threads**. A thread is one chat with an agent. Start a new one with the pen icon at the top of the sidebar; rename, archive or delete one with a right-click.

The sidebar lists every project's threads together, like T3 Code: **pinned** threads on top, then the **active** ones, newest first. At the bottom are shelves you open and close: **Working** (turns running now; a thread that finishes or asks for an approval goes back to the top), **Snoozed** and **Settled**. Hover a thread to **Settle** it when you're done with it, or to snooze it; the open thread then moves on to the next one. Threads with no activity for a week settle by themselves (Settings → Sidebar: turn it off or pick 1 to 90 days), and writing in a settled thread brings it back. The folder button next to Search lists one project's threads: click a project, Shift+click to add another, **All projects** to list them all again. It also has **Add project**, and each project's pin, settings and right-click menu. Prefer a folder per project? Turn on **Group threads into project folders** in Settings.

Resolve has one project open at a time, so Linker-Edit follows it:

- A thread only changes its own project. When Resolve has another one open, sending a message saves that one and opens the thread's project, and **Open it again** brings the other one back. **Open in Resolve** switches without a message. A Blackmagic Cloud project you open in Resolve yourself.
- **You're asked first when you may be working in the open project.** Resolve doesn't say whether a project was changed, so Linker-Edit watches it: when the page, the playhead, the timeline or its clips changed in the last 15 minutes, or the project was just opened, you get **Save and switch** or **Not now**. In Supervised mode you're always asked. What it can't see are changes that leave those alone, such as a grade.
- It never leaves a project that is rendering, or one with work in it that was never saved.
- One thread works in Resolve at a time. The others wait until it's done.
- When Resolve has a project open that isn't in Linker-Edit yet, the sidebar offers to add it.

**Starting Resolve:**

Resolve doesn't have to be running when you send a message. When the task needs it, Claude or Codex starts it on the edit computer with `start_resolve`, waits until it's ready (up to a few minutes), and opens the thread's project. Closing Linker-Edit leaves Resolve running.

- With another project open, the same questions are asked as when you send a message.
- A Blackmagic Cloud project, or a project not linked to Resolve yet, you open in Resolve yourself after it has started.
- If you stop the turn, Resolve still finishes starting.

**Open in Resolve** starts Resolve too.

**Looking at another project:** say "use the same intro as in Promo", and the agent visits that project: Resolve saves yours, opens the other, and the agent reads its timelines, media and frames. Every change is refused there, including scripts. After that, or when the turn ends or you stop it, Resolve goes back to your project. It only visits projects you've added to Linker-Edit, and neither to nor from a Blackmagic Cloud project. To take something along, it exports a timeline there and imports it in your project.

**Adding a project** (**Add project** in the folder button's menu). With more than one computer, first choose the one the project is on: its threads run there, with that computer's Resolve. Then:

- **New:** a new, empty project. Resolve starts when it isn't running, saves and closes the project it has open, and makes the new one in the database it has open, local or network. Agents can't make projects, and in Blackmagic Cloud you make it yourself.
- **Open in Resolve:** the project Resolve has open now.
- **Database:** any project in the database Resolve has open, local or network (PostgreSQL). To add one from another database, switch to that database in Resolve's Project Manager first.
- **Cloud:** a Blackmagic Cloud project, by its name. You open it in Resolve's Project Manager yourself, and Linker-Edit picks it up once it's open: [Blackmagic Cloud's terms](https://static.cloud.blackmagicdesign.com/html/terms) don't allow scripts to use the service, so Linker-Edit never opens, creates, uploads or syncs cloud projects. If it's open in Resolve now, use **Open in Resolve** instead.
- **Folder:** a project folder from another computer, such as a teammate's edit history you cloned.

Folder fields list the folders as you type, like T3 Code's **Add project**. Use ↑/↓ to choose a folder, and Enter or Tab to open it. Enter with no folder chosen picks the path you typed. Project folders are marked **Linker-Edit project**. The folders listed are on the project's computer, even when you use it from another one. For a project on the computer in front of you, **Browse…** in the desktop app also opens the system's folder dialog.

A project with no Resolve project yet links itself to the one open in Resolve with its first message. To link it to another one later, open that one in Resolve and use **Project settings**. Projects stay linked when you rename them in Resolve.

Each project folder has a `linker-edit.json` that says which Resolve project it belongs to. It's committed with the edit history, so a teammate who clones it gets the link. What differs per computer, such as the database name and the cloud media folder, stays in `.linker-edit/local.json`, which Git ignores. Removing a project from Linker-Edit keeps its folder.

## Edit history

Each project folder is a Git repository. Linker-Edit saves the open Resolve project in it:

- **After every turn in which the agent changed Resolve.** The commit is authored by Claude or Codex and holds your prompt, the Resolve actions and a summary such as `"AI - Rough cut": 3 clips removed, length 4:02 → 3:41`.
- **Before every turn, if you changed something in Resolve yourself.** The agent can tell your edits from its own. In a Blackmagic Cloud project, these saves are labelled as changes made outside Linker-Edit, since a collaborator may have made them.

Each timeline is a folder under `resolve/<project>/` with `cuts.json`, `audio.json`, `markers.json` and `metadata.json`, one clip per line, so `git log -p` reads like an edit decision list. Media is never stored.

**History** in the top right lists the project's saves, newest first, with what changed per timeline; saves made in the open thread are marked. **Restore** on a save writes the undo request, naming its timelines, into the composer for you to send.

Ask "what did you change?" or "undo the last change". The agent reads this history and rebuilds the earlier version with the `restore_timeline` tool, in one step: a new `AI - … (restored …)` timeline with the saved size, frame rate, clips on their tracks, clip settings, disabled clips and markers. When the saved timeline still exists, the restored one can replace it with **Use as main timeline**. It never edits or deletes a timeline to undo. Color grades, effects, titles and generators aren't saved yet; the tool lists what it left out.

You can share the history like any Git repository (`git remote add`, `git push`), but merging two people's histories back into Resolve isn't supported yet.

## Long chats

Everything said and every Resolve result stays in the agent's context for the rest of a chat, and each turn rereads it. Linker-Edit keeps that in check:

- **Context meter.** With Claude, the ring next to the send button shows how full the context is. Click it for the token count and **Compact now**, which replaces the chat so far with a summary. You can keep chatting afterwards.
- **Compact on return.** Claude keeps a recent chat cached for about an hour on a subscription (5 minutes with an API key). If you come back to a chat over 200,000 tokens after that, a **Compact** chip shows next to the send button: sending then summarizes the chat first, so all of it isn't reread at full price. Click the chip to switch to **Full** and send with the whole chat.
- **Long Resolve results are shortened.** A Resolve result over about 5,000 tokens, such as every item on a long timeline, has its lists cut down before the agent reads it. The agent is told to ask for a narrower part if it needs more.
- **Threads survive restarts.** Each thread is kept in `.linker-edit/threads/` in its project folder, which Git ignores, and the agent continues the same conversation. If it can't (say Claude's session was deleted), it starts a new one with a summary of the chat.
- **Switching agent keeps the chat.** The new agent gets a short summary of it with your next message: what was asked, what was said, and what changed in Resolve. It doesn't get the other agent's full context.

Codex has no context meter or compaction here: its SDK doesn't report how full the context is or run `/compact`.

## Usage

**Usage** (the chart icon at the bottom of the sidebar, or Ctrl+U / ⌘U) shows what Claude Code and Codex used on the computer of the thread you have open, as in T3 Code. It counts every session of theirs on that computer, Linker-Edit's own chats and any you ran elsewhere, from the transcripts they keep in `~/.claude/projects` and `~/.codex/sessions` (or `CLAUDE_CONFIG_DIR` and `CODEX_HOME`).

- **Cost** and **Tokens** show the past 24 hours or 7, 30 or 90 days: the total, each agent's share, a chart per day or hour, the token mix and a table by model or by day. Cost is what the same tokens would cost on the API, from [LiteLLM's public price list](https://github.com/BerriAI/litellm), not your subscription bill. A model the list doesn't know shows as **Unpriced**.
- **Limits** shows how much of your Claude and ChatGPT plan's session and weekly allowance is left, and when it resets. The line in each bar is where even spending would be. Reading them sends no prompt, so it costs nothing; an API key has no limits to show.
- **The filter** after the computer's name hides Claude or Codex.
- **Keys** on the page: C, T and L switch between Cost, Tokens and Limits, Ctrl+Shift+1 to 4 (⌘⇧1 to 4) pick the period, and Esc closes it.

Unlike T3 Code, it covers one computer at a time, and there's no per-model detail or custom price list.

## Preview

**Preview** in the top right plays a timeline beside the chat, in two ways:

- **Cut preview** starts right away. It plays your media straight from disk, following the cuts, with zoom, position and opacity. Color, effects, transitions and titles only show in a render, and formats browsers can't decode (ProRes, BRAW and similar) show as a placeholder. It also plays earlier versions: **Play** on a timeline in the history opens it as it was in that save.
- **Full render** has Resolve render the exact picture as a small MP4, and the player switches to it at the same spot. While it renders, Resolve switches to that timeline and the Deliver page, then puts your timeline, page and render settings back. The agent waits until the render is done, and you can stop it. Only the timeline as it is in Resolve now can be rendered.

Keys: Space play or pause, ← → one frame (Shift: one second), ↑ ↓ previous or next cut, F full screen, M mute.

**Give feedback at this moment** starts a message with where the playhead is ("At 0:14 (frame 336) in …"), so the agent knows which spot you mean.

## Teaching it a brand

A **brand** is an org you're in on Linker: sign in with Linker and each of your orgs shows as a brand. Inside a brand you make **sub-brands**, such as a channel with its own audience, and **series**, the formats, such as a weekly vlog or reels. A series goes in the brand or in one of its sub-brands. Each has rules, kept in a `SKILL.md` file, that the agent follows in every project in it, brand first. You can write them yourself, or have the agent work them out from videos and Resolve projects you've finished. The aim: you give it footage, it makes the first cut, and you only give feedback on the video.

The **Brands** button next to Settings, at the foot of the sidebar, opens every brand with its sub-brands, series and projects, where you make, rename, move and delete sub-brands and series and write their rules. **Add project** asks for a brand, then a sub-brand or series in it. Workspaces from an earlier version are listed under **Not in a brand** until you move them into one. A brand whose org you left on Linker is greyed and takes nothing new.

**Set up with AI** on a brand, sub-brand or series opens a setup thread. It runs on your own Claude or Codex, and it changes nothing in Resolve.

1. Point it at your **finished Resolve projects**, a folder of **finished videos**, or both, and if you have them the **raw footage** some were cut from and the brand's **assets**.
2. It visits the projects one by one, read only, and looks at what a rendered video can't show: how the tracks are used, shot lengths, titles, where music starts and stops. They have to be [added to Linker-Edit](#projects-and-threads) first. You're asked before Resolve leaves a project you're working in, and it goes back to that one afterwards.
3. It measures each video with the ffmpeg Linker-Edit comes with: length, aspect ratio, shot lengths, how quiet it gets, loudness, how fast people talk and how it opens. It looks at a contact sheet of its frames and reads the transcript. Measurements are kept in `~/LinkerEdit/cache/study` until a file changes.
4. With raw footage, it lines up what is said in the finished video with what is said in the raw files. That shows what the editor kept and cut, which take they used, and whether they reordered.
5. It tells you what it found, with the numbers, and asks only what it can't see: who watches, where the music comes from, where the examples disagree.
6. It proposes rules for what it sets up and for each series. You see what changes and **Accept** or **Decline**. Nothing is saved without that.

`SKILL.md` is in the Agent Skills format, so you can use the same folder as a skill in Claude Code or Codex. A `RULES.md` from an earlier version becomes the `SKILL.md` the first time something in the brand, sub-brand or series is saved.

Its notes on each example are kept as files next to the rules, in `~/LinkerEdit/collections/<name>/examples`.

**It is laid out like code.** A project is a repository of one video, and it gets its rules from the folders of its brand, sub-brand and series:

```
~/LinkerEdit/collections/ferret-media/   the brand
  SKILL.md                               what is always true for the brand
  examples/                              the agent's notes on example videos
~/LinkerEdit/collections/reels/          a series in it
  SKILL.md                               how this kind of video is made
~/LinkerEdit/projects/Reel 14/           a project in that series: a Git repository
  PROJECT.md                             this video: brief, footage, decisions, feedback, status
  resolve/                               its timelines as text, one save per turn
  AGENTS.md                              written by Linker-Edit: everything above in one file, for the agent
```

The agent reads these files from the most general to the most specific, and the later one wins. It keeps `PROJECT.md` itself, so a new thread knows what the video is about, what is in the footage and what you already said. You can read and edit it under **Project file**, next to Preview and History, and each change is saved in the project's edit history. Rules set up with AI use the same headings for every brand. **Must** holds what can be measured, such as length and aspect ratio, and the agent checks its cut against it before it shows you.

**New video**, in an empty thread of a project that has no `PROJECT.md` yet, takes a title, the footage folder and, if you have them, a concept and a script. A project without a brand can start this way too. Linker-Edit writes them to `PROJECT.md` and `SCRIPT.md` in the project's folder, which are in its edit history, and the agent starts. It never changes your script.

That project then **works on its own**, which is also a switch on a brand, a sub-brand, a series and in the project's settings. With it on, a new thread starts with New video, and the agent makes the whole first cut without asking questions: it imports and transcribes the footage, builds the cut the way the rules describe, checks its own work, renders it and measures the file against the rules' **Must** lines (length, aspect ratio, loudness, quiet stretches), and ends with a note of what it made, what it left out and what it decided for you. It still only changes `AI - …` timelines, and you still make one the main timeline yourself.

**It learns from your feedback, and you check what it learned.**

- When you give feedback, the agent first says what it understood, then fixes it.
- Feedback that will hold for the next video too becomes a **lesson**. The agent only proposes one after you've seen the fix, and shows it next to your own words. Decline it if that isn't what you meant.
- When you press **Use as main timeline**, the agent compares its first version with the one you approved, including what you changed in Resolve yourself, and proposes lessons from the difference.
- Lessons are listed under the rules on the Brands page, each with the words it came from, and you can remove any of them. It also shows how many rounds of feedback each approved edit took, so you can see whether the rules work.

What it can't do: it sees still frames, not motion, and it can't hear. It knows how loud a video is and where it's quiet, not whether a song fits. So formats driven by speech (talking head, interview, podcast, reels cut from longer talk) work best.

### Skills

A skill says how one kind of work is done, such as color grading. The agent sees a list of them and reads one when the work needs it, so they don't fill every chat. Linker-Edit brings five, for every project, with or without a brand:

- `video-editing`: from a brief or footage to a checked first cut. It has a file for a talking head, an interview, a scripted video and a short vertical one.
- `cutting-speech`: retakes, silences, filler words.
- `color-grading`: correct, match, then a look.
- `sound`: choosing music, levels, loudness.
- `titles-and-captions`: what to put on screen, and where.

Bring your own skills by dropping them on the **Skills** card, or with **Import**: your own, for every project, in **Settings → Skills**; a brand's, sub-brand's or series' on the Brands page. Drop one `SKILL.md`, a skill's folder, a folder of many skills, or a `.zip` or `.skill` export, such as Claude's. Every `SKILL.md` makes its folder a skill, with its `references/`, `scripts/` and other text files; a loose `.md` file becomes a skill by itself. A skill of the same name is replaced whole. Hidden files, binaries and files over 256 KB are left out, and the app says which and why. One drop can be up to 8 MB.

Each skill is a folder with a `SKILL.md` in the Agent Skills format, so you can also put one in place by hand:

```
~/LinkerEdit/skills/color-grading/SKILL.md                     for all your projects, with or without a brand
~/LinkerEdit/collections/ferret-media/skills/color-grading/    for a brand
~/LinkerEdit/collections/reels/skills/color-grading/           for a series
```

Skills of the same name add up. The agent reads all of them, and the most specific one wins where they disagree, so a series' skill only says what is different. The `description` in its front matter tells the agent what work it is for; an imported skill without one gets its first paragraph, and one that only adds to a skill of its name needs none.

## Library

**Library** in the top right (or in Settings) opens the folders you reuse: music, sound effects, graphics, LUTs, footage, anything else. Linker-Edit only points at them. It never copies, moves or deletes a file in them.

- **Universal or this project.** Folders in the universal library are for every project, and are kept in `~/LinkerEdit/library.json`. A project's own folders come first for that project, and are kept in its `.linker-edit/local.json`, on this computer only.
- **Read once.** Adding a folder reads each file's length, channels and loudness (LUFS) with the ffmpeg Linker-Edit comes with. That's kept in `~/LinkerEdit/cache/library`, and **Rescan**, or starting the app, only reads files that are new or changed.
- **Notes for you and the agent.** A folder's note ("licensed for YouTube only") goes to the agent with every file from it. Star files, and give them tags and notes, to find them again.
- **The agent looks here first** for music, sound effects and graphics (`library_search`), and brings what it picks into a `Library/<Kind>` bin (`library_import`). A file already in the media pool is used again, not imported twice, and the library notes which project and timeline used it.
- **Plays in place.** The page plays files from the folder itself. Only files inside library folders are served, to this computer and your own devices over remote access.

## Transcripts

Ask for anything that depends on what's said, such as "transcribe the interview clips" or "cut the bits where I mention pricing". The agent transcribes the clips it needs with [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (`large-v3-turbo`) on this machine. Nothing is uploaded, and it costs nothing.

- **The speech engine installs itself** into `~/LinkerEdit/tools` when the app starts: about 1.5 GB on Windows with an NVIDIA GPU, or 0.9 GB without one, where it runs on the CPU and is much slower. On a Mac it's about 0.9 GB and runs on the GPU through Metal on Apple silicon, or on the CPU on an Intel Mac. whisper.cpp has no Mac download of its own, so Linker-Edit builds one (`.github/workflows/whisper-macos.yml`). On Linux it downloads a CPU build. On Linux and macOS a `whisper-cli` already on your PATH comes first, such as a GPU build or Homebrew's. The sidebar shows the progress with **Cancel**, and offers **Set up** to try again if the download failed.
- **It's fast.** On an RTX 2060, a 3-minute clip takes about 9 seconds, model loading included.
- **Each transcript is kept** in `~/LinkerEdit/transcripts`, as text and as JSON with per-word times, until the clip's file changes. Asking again is instant.
- **Languages are detected per clip** by sampling it from start to end, so a clip that opens in English and goes on in Dutch is transcribed as Dutch, with the English parts kept in English. If you know the language, say so in your prompt.
- **Transcripts in Resolve itself:** ask for it, and the agent uses Resolve's own transcription instead. It stores the transcript on the clip, where you can edit by text. It never adds subtitles to a timeline unless you ask.

## Your computers

The sidebar lists the projects of all your computers, like T3 Code with several environments. A project lives on one computer: its threads run there, with that computer's Resolve, footage and agents, and two threads on two computers can work at the same time. With more than one computer, each thread shows the name of the computer it's on, and a computer icon next to each project in the folder button's menu says which one it's on (hover it for the name).

When a computer is offline, its threads stay in the sidebar, greyed and marked *offline*, as they were when it was last seen. You can't open or change them until it's back. The sidebar's foot shows Resolve and updates of the computer whose thread is open; Settings shows each computer's own.

Your other computers show up through Linker Connect or Tailscale, below.

## Linker Connect

Open your edit machine from your other computers, from anywhere, with your [Linker](https://linker.cool) account and nothing else to set up. It works like T3 Code's T3 Connect: Resolve, your footage and the agents stay on the edit machine, and your other computers show it live.

1. On the edit machine, sign in with Linker, then turn on **Linker Connect** in Settings → Remote access. It gets its own address, like `https://edit-0123456789abcdef.linker.cool`.
2. On the other computer, run Linker-Edit and sign in with the **same** Linker account. The edit machine's projects show up in its sidebar next to its own.

- **Up to 3 edit computers per Linker account.** The computers you only work from don't count. Unlink one in the same settings to make room.
- **Only your account gets in.** The edit machine asks Linker who a sign-in belongs to, and only lets in the account it was linked with. Each computer that opened it is listed under *Signed-in devices*, where you can sign it out at once; the iPad app, once it's out, shows there as a *Native app*. A sign-in lasts 30 days. Turning Linker Connect off, or signing out of Linker on the edit machine, signs every device out.
- **Linker makes the tunnel, not the traffic.** Linker only gives the edit machine a [Cloudflare Tunnel](https://developers.cloudflare.com/cloudflare-one/connections/connect-networks/); chats, previews and media go through the tunnel straight to your edit machine. Linker-Edit runs `cloudflared` for it: one on PATH, or a pinned release it downloads once (20 to 55 MB) and checks by hash.
- **It stays on** across restarts, and is reachable while Linker-Edit runs on the edit machine.
- **Tailscale only, if you like.** Turn off **Use the tunnel** and the edit machine stays linked to your account without an address at Linker: your devices reach it over Tailscale (below), and it still counts toward the 3.

See [docs/sync-plan.md](docs/sync-plan.md) for how it's built.

## Remote access over Tailscale

Chat from another computer, such as a laptop on the couch or a PC at the office, while Resolve, your footage and the agents stay on your edit machine. It goes over your own [Tailscale](https://tailscale.com) network, so nothing is opened to the internet.

1. Install Tailscale on both computers and log in to the same account.
2. On the edit machine, open Settings → Remote access, then **Turn on** under Tailscale.
3. On the other computer, run Linker-Edit too. The edit machine's projects show up in its sidebar next to its own, and their threads run on the edit machine.

Only your own computers show up, and only once remote access is on and Linker-Edit is running on them. To update one, open its settings (choose it at the top of Settings) and use **About & updates**: it restarts Linker-Edit there into the new version. The address in the dialog, like `https://studio.tail1234.ts.net:8443`, also works in a browser on your devices, such as a tablet.

- **Only your devices get in.** Linker-Edit lets in devices logged in to the Tailscale account that owns the edit machine. Other people on a shared tailnet and tagged devices are turned away.
- **The first time, Tailscale may ask you to allow it.** If Tailscale Serve or HTTPS certificates are off for your tailnet, the dialog shows Tailscale's link to turn them on. Then click **Turn on** again.
- **It stays on** after Linker-Edit or the computer restarts, until you turn it off in the same dialog. While Linker-Edit isn't running, the address shows an error.
- **It uses port 8443**, so it doesn't take over an address you already serve on 443, such as T3 Code's. If 8443 is taken too, set `tailscalePort` in the config.

### Your own web address

[![Deploy to Cloudflare](https://deploy.workers.cloudflare.com/button)](https://deploy.workers.cloudflare.com/?url=https://github.com/FerretMedia/linker-edit/tree/main/apps/web)

Put Linker-Edit on your own free Cloudflare address and use your edit machines from any browser: sign in with Linker, allow the address once, and you're in the app with the projects of all your computers. It reaches each one over Tailscale when it can and through Linker Connect otherwise. **Sign out** is in Settings and at the bottom of the sidebar; see [apps/web](apps/web/README.md).

## Sign in with Linker

Optional. Press **Sign in with Linker** at the bottom of the sidebar and sign in with your [Linker](https://linker.cool) account in your browser. The sidebar then shows who you are and your plan. Nothing else changes yet: this is the start of working on your Linker projects from Linker-Edit (see [docs/linker-plan.md](docs/linker-plan.md)).

- **Sign in on the edit machine itself.** Linker sends your browser back to Linker-Edit on `127.0.0.1`, so it works on the computer Linker-Edit runs on, on the default port 4317.
- **Linker-Edit asks for your name and email,** and to stay signed in. It keeps the sign-in in `~/LinkerEdit/linker-account.json`, readable only by you. **Sign out** in the account's menu removes it and tells Linker to forget it.

## Testing

- `npm test`: unit tests. Free and offline. With Python installed, they also run every Resolve script (`apps/desktop/server/resolve/scripts`) against a stand-in Resolve, and the frame and silence tools against media made with ffmpeg.
- `npm run test:e2e`: drives the built app in a browser (`npx playwright install chromium` once). Claude is replaced by a recorded turn, so it costs nothing.
- `LINKER_EDIT_REPLAY=server/providers/fixtures/claude-turn.jsonl npm run dev`: the same replay, for UI work without spending tokens.
- `npm run record-fixture -- <claude|codex> <out.jsonl> "<prompt>"`: records a real turn on the cheapest models, for new provider tests.
- `node apps/desktop/scripts/update-smoke.ts`, after `npm run build && node apps/desktop/scripts/build-desktop.ts --dir`: starts the unpacked desktop app against an update feed on your computer and checks that it downloads the update, stops its server and quits to install it, and says so when the install didn't take. On Linux, run it under `xvfb-run`.
- `RECORD_IPAD_FIXTURES=1 npm test -- scripts/ipad-fixtures.test.ts`: records what the server sends over `/ws` into the iPad app's test fixtures. Run it after changing `shared/protocol.ts`; the iPad CI then checks the Swift models still read every message.
- `npm run test:live -- --project "<scratch project>"`: runs real editing prompts against Resolve with approvals off, then checks that existing timelines and media are untouched. It then compacts the chat and continues it in a new session, as after a restart. It costs tokens; run it before releases and after Resolve updates. `--dry-run` only checks that it can see the project.

## Contributing

Linker-Edit is built in the open, and it's meant to stay small and obvious. The most useful contributions right now are:

- **Bug reports from real edits:** what you asked for, what the agent did, and what went wrong.
- **Workflow ideas:** editing tasks you'd hand to an assistant if you could.
- **Fixes**, especially to the Resolve integration and the guardrails.

For anything larger, please open an issue before writing the code, so we can agree on the approach.

## Credits

MIT licensed. Speech to text by [whisper.cpp](https://github.com/ggml-org/whisper.cpp) (MIT) with OpenAI's Whisper model, and audio decoding by [FFmpeg](https://ffmpeg.org) through `ffmpeg-static` (GPL), both run as separate programs. Linker-Edit drives Resolve through the MCP server Blackmagic Design ships with DaVinci Resolve Studio; it starts your installed copy and doesn't distribute it. The edit history format follows [Vit](https://github.com/LucasHJin/vit). The interface is modelled on [T3 Code](https://github.com/pingdotgg/t3code); its theme tokens, provider icons and usage page are adapted from T3 Code, MIT licensed, Copyright (c) 2026 T3 Tools Inc. Built with SvelteKit. DaVinci Resolve is a trademark of Blackmagic Design; Linker-Edit is not affiliated with Blackmagic Design.
