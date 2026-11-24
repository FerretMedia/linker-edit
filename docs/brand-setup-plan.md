# Plan: the AI learns a brand, then edits on its own

The goal: you point Linker-Edit at a brand's example videos and project folders, the AI works out how that brand is edited, and from then on it makes a first cut without being guided. You only give feedback on the video.

## Where we are (main, b43861a)

- A workspace and a series each have a `SKILL.md`. The agent gets them on every turn, from the most general to the most specific (`apps/desktop/server/rules.ts`, `agentRules`). You write them by hand, up to 20,000 characters.
- The library points at folders of music, sound effects and graphics. The code already takes a chain (universal → workspace → series → project), but a workspace or series can't have its own library folders yet (`CollectionLibraryFolder = never`).
- The agent can already study footage: `transcribe` (local, with word times), `find_silences`, `view_frames` on a media file, `detect_scene_cuts` on a timeline.
- The edit history saves every timeline as JSON in Git, with the agent's turns and your own changes in Resolve as separate commits.
- Rule 6 tells the agent to ask when the brief is ambiguous. That is the opposite of working on its own.
- A thread always belongs to a project, and a project to a Resolve project. A workspace has no thread of its own.

## The idea in one picture

```
 Setup (once per brand)            Every video                     After every video
 ──────────────────────            ───────────                     ─────────────────
 point at examples      ──►  drop footage, pick the series  ──►  you give feedback
 AI studies them              AI makes a first cut alone          AI fixes the cut
 AI asks what it can't see    AI checks its own work              AI proposes a change
 you approve the brief        AI shows the video + its notes      to the brief, you accept
        │                                                                  │
        └──────────────────── the brief ◄──────────────────────────────────┘
```

Three parts: **learn** the brand, **edit alone**, and **get better from feedback**. The third is what makes the second one work after a few videos, so it isn't optional.

## Part 1: what the AI has to know (the brief)

An editor who joins a brand learns these things. The brief is this list, filled in:

| Section | What it holds | Where it comes from |
| --- | --- | --- |
| Who and where | Audience, platforms, language, tone | You (interview) |
| Formats | Per kind of video: aspect ratio, length, frame rate | Measured |
| Structure | Hook, intro, segments, call to action, outro, and how long each is | Transcripts + you |
| Pacing | Average shot length, cuts per minute, how long a pause may be | Measured |
| What gets cut | Retakes, filler words, tangents, swearing; which take wins | Raw vs final (see below) + you |
| Sound | Music or not, what kind, where it starts and stops, sound effects | Partly measured, mostly you |
| Graphics | Titles, lower thirds, captions, logo, bumpers: look and placement | Frames + you |
| Color | The look, a LUT if there is one | Frames + you |
| Assets | Where the logo, intro, music and LUTs are | You point at folders |
| Footage | How a shoot folder is laid out: cameras, audio, B-roll | The project folders |
| Delivery | Render preset, file name, folder | You |

### It maps onto what exists

- **Workspace = the brand.** Who and where, sound, graphics, color, assets.
- **Series = a format.** Structure, pacing, length, delivery. "Weekly vlog" and "Reels" are two series of one brand.

So there is no new kind of group. This also covers most of what "styles" (thread 5 of the library plan) were for.

### Where it's kept

The brief is in the workspace's or series' own folder:

```
collections/<workspace>/
  SKILL.md              # the rules: style, and for a series the steps of one edit. In every prompt (as now)
  collection.json       # also holds the lessons from feedback, and whether it works on its own
  examples/<video>.md   # the AI's notes on one example
```

### The structure: like a code repository

A project is the repository of one video. It gets its rules from the folders of its series and its workspace, the way a code repository gets instructions from the folders above it. Each level answers one question:

| Level | Question | Changes | File |
| --- | --- | --- | --- |
| Workspace (brand) | What is always true? | Rarely | `collections/<workspace>/SKILL.md` |
| Series (format) | How is this kind of video made? | Sometimes | `collections/<series>/SKILL.md` |
| Project (one video) | What is this video? | Every session | `projects/<project>/PROJECT.md` |

```
projects/Reel 14/       # a Git repository (the edit history, in Vit's format)
  PROJECT.md            # the video: Brief, Footage, Decisions, Feedback, Status. The agent keeps it
  linker-edit.json      # which Resolve project, and which series it is in
  resolve/              # its timelines as text, one commit per turn
  AGENTS.md             # generated before each turn: Linker-Edit's rules, then the three files above,
                        # each under a heading with the path it comes from
```

- **The project file is the agent's, the brand's files are yours.** The agent writes `PROJECT.md` without asking: it is about one video, and every change is a commit you can read and undo. Rules for a workspace or series still always ask.
- **Fixed headings.** Workspace: Who, Voice, Must, Look, Sound, Assets. Series: Must, Structure, Style, How an edit goes. **Must** holds only what can be measured on a finished video, and the agent checks its cut against each line. Taste goes under the others, and is taught mostly by the examples.
- **Later wins.** Project over series over workspace. Linker-Edit's own rules (only `AI - …` timelines change, nothing is deleted) can't be lifted by any of them.
- **One name for both agents.** Not `CLAUDE.md`: Codex wouldn't read it. Linker-Edit puts the three files in `AGENTS.md` for Codex and gives Claude the same text.
- A project made before `PROJECT.md` keeps its `RULES.md`; both are read.

Examples of what each level holds:

| Kind of work | Series | Project |
| --- | --- | --- |
| Reels | 9:16, 30 to 40 s, question in the first 2 s, a cut every 2 s | The topic, the must-have shot, which take of the intro is cleanest |
| Podcast | Camera switching, intro and outro, how clips are picked | The guest, timestamps of good moments, what to cut for privacy |
| YouTube long-form | Hook, B-roll rhythm, chapter style | The outline, the B-roll list, which parts ran long |
| Client work | The client's format and deliverables per platform | The client's brief, their feedback per round |

`SKILL.md` and the lessons go into every prompt, so they stay short. `PROJECT.md` does too: the agent is told to rewrite it, not append, and to keep it under 4,000 characters. The notes on examples don't: the agent is told where they are and reads one when it needs a reference. The measured numbers of each video are cached in `~/LinkerEdit/cache/study`.

## Part 2: setup, with the AI

It runs on your own Claude or Codex subscription, as a normal agent turn. No extra API key.

### Step 1: point at things

On a workspace or series in Settings: **Set up with AI**. You give it:

1. **Finished videos** (a folder, or files). The most important.
2. **Raw footage** some of them were cut from.
3. **Brand assets**: logo, intro, music, LUTs, fonts. The brief names them by their path.

### Step 2: the AI studies them

A new tool, `study_media`, does the measuring without the model, so it's cheap and repeatable:

- Duration, aspect ratio, frame rate (ffprobe).
- Shot lengths and cuts per minute (ffmpeg scene detection).
- Loudness (ffmpeg `ebur128`).
- Speech and silence: how much is talk, the longest pause that was left in.
- Transcript (whisper, local): words per minute, and the text for structure.
- A contact sheet: one image of many small frames, so the model sees the look of a whole video in one picture instead of 40.

Results are cached per file like transcripts. The model then reads the numbers, the transcript and the contact sheet, and writes `examples/<video>.md`. One example per subagent, so twenty videos don't fill one context.

**Raw vs final** is the best source when you have both. Transcribe the raw footage and the finished video and line the words up. That shows exactly what the editor cut: the retakes, the ums, the tangents, how much of a long answer survived. No project file needed.

**Past Resolve projects** would be even more exact: tracks, titles, zooms and markers as they were made. The setup thread visits the ones added to Linker-Edit, read only, and Resolve goes back afterwards.

### Step 3: the AI shows what it found and asks

It reports with evidence, then asks only what it can't see or where the examples disagree:

> Your 8 reels are 28–41 seconds, 9:16, a cut every 1.9 seconds. All open on a question in the first 2 seconds. Captions are white with a yellow highlighted word, bottom third. Music runs under everything.
>
> 1. Two reels end on the logo, six don't. Which is the rule now?
> 2. Where is the music from? I can't tell from the video.
> 3. Three videos have the retakes cut very tight (0.1 s), the rest are looser. Which do you prefer?

Each question comes with a few options to pick from.

### Step 4: you approve the brief

The AI proposes the rules of the workspace and of each series. You see what changes and accept or decline. Nothing is written without that; afterwards you can edit them in Settings.

### Step 5: a test edit

With a project in the series and the raw footage of one example, the AI edits that footage blind, renders it, and compares both the real video and its own render with the raw footage (`compare_cut`). Your feedback on it becomes the first lessons. This is how you find out the brief is good before a real job depends on it.

### Where setup runs

In a setup thread of the workspace or series. It is kept as a project of its own with no Resolve project, called "<name> setup" in the sidebar, so it reuses the chat, the threads and the agents as they are. It never links to Resolve, and tools that work in Resolve are refused in it, except for reading a project it visits.

## Part 3: editing on its own

A workspace or series gets a switch: **Works on its own**. With it on:

1. **New video**: a new thread in one of its projects asks for the footage folder. That is the whole brief for the job. A line of notes is optional ("the second interview is the important one").
2. The agent follows the series' rules, which end with the steps of one edit: import and sort the footage → transcribe → pick the material → rough cut → tighten → music, sound effects, titles from the library → render a preview.
3. **It checks its own work** before showing it: length against the format, frames of every title and the first and last frame, no black frames, no cut inside a word. It fixes what it finds.
4. It hands over the preview with an **editor's note**: what it made, what it left out and why, and the guesses it made instead of asking ("I used the take from 12:40, the earlier one had a plane overhead").

Rule 6 changes for these series: don't ask, decide from the brief, and list the decisions afterwards. Rules 1 to 5, 7 and 8 stay. It still only touches "AI - …" timelines and you still make it the main timeline yourself.

## Part 4: getting better from feedback

- **Feedback on the video.** "Give feedback at this moment" in the preview starts a message with the time and frame of the playhead.
- **Lessons.** When feedback will hold for the next video too, the agent proposes a lesson: one short rule.
- **What you changed yourself.** The edit history already records your changes in Resolve as their own commits. When you press "Use as main timeline", the agent compares its first version with the approved one and proposes lessons from the difference.
- **One number per series:** rounds of feedback until approved, shown in Settings. If it doesn't go down, the brief isn't working.

### When the AI misunderstands your feedback

A lesson from a misunderstood comment would spoil every later edit, so four things stand in the way:

1. The agent first says in one line what it understood, then makes the change.
2. It only proposes a lesson after you've seen the fix, so the fix has already shown whether it understood.
3. You see each lesson next to your own words, and accept or decline it. This asks in full access too. Nothing is learned silently.
4. Lessons stay listed in Settings with the words they came from, and you can remove any of them.

## What it can't do yet

Better to know before building:

- **It can't hear.** It knows loudness and where there is speech, not whether a song fits or a mix sounds right. Music choice comes from library tags and your rules.
- **It sees stills, not motion.** A shaky shot or a bad transition can slip past it.
- **Titles are simple.** `add_title` makes text; motion graphics templates aren't reachable yet.
- **Color and effects** aren't in the edit history, so it can't learn those from your changes.

So the first formats that will really work alone are the ones driven by speech: talking head, interview, podcast, reels cut from longer talk. B-roll-driven and music-driven edits come later.

## What is built, and what isn't

Built:

- `study_media` (measurements and contact sheets) and `compare_cut` (raw against final by transcript).
- Setup threads with **Set up with AI** that also visit finished Resolve projects, read only, and the brief tools `save_brief`, `add_lesson` and `save_example`, which the user accepts or declines.
- **Works on its own** per workspace or series, with the footage form in a new thread.
- Lessons from feedback, the retro on "Use as main timeline", rounds until approved, and feedback at a moment of the preview.
- `PROJECT.md` per project, kept by the agent and saved in the edit history, and the rules' headings naming the file each comes from. **Project file** in the app shows it and lets you edit it.
- The measured check: working on its own, the agent renders its cut and measures it against Must before showing it.

Not built:

- A footage log of its own. What the agent finds in the footage is a section of `PROJECT.md`; long shoots will need a file per shoot that isn't in every prompt.
- A **Must** the code can read. The agent working on its own is told to render its cut, measure it with `study_media` and compare each Must line, and to report the numbers. The comparing is the agent's: Must is plain text, so Linker-Edit can't refuse a cut that breaks it.
- Folding old lessons into the rules when there are many.
- Library folders on a workspace or series (thread 4 of the library plan). Until then the brief names the brand's assets by their path, and the universal library holds the music.
- Questions as cards with choices: the agent asks numbered questions with options in the chat.
- Approved videos becoming examples by themselves: the agent can save notes on one when asked.

Not checked against a real Resolve or a real agent yet: see the PR.

## Decisions (2026-11-17)

1. **Setup is a thread**, not a form.
2. **Raw against final is in from the start**: the user can supply raw footage with the examples.
3. **Works on its own is per workspace or series.**
4. **Lessons always ask.**
5. **Laid out like a code repository**: a project is a repository with its own `PROJECT.md`, under the files of its series and workspace.
