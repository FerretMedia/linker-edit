---
name: video-editing
description: From a brief, a script or footage to a first cut, and changing a cut after feedback. Read it before you plan or build any edit, also a short one. It says which other skills to read and when.
---

# Video editing

A cut is good when a viewer who knows nothing about the footage follows it and stays. Decide for that viewer.

## 1. Take stock

- Read the brief: PROJECT.md and SCRIPT.md in the working folder, when they are there. The user's rules say what the brand or series always does.
- `import_media` what isn't in the media pool yet, sorted into bins, and list it (`list_media`).
- `transcribe` everything with speech, in one call. Read it whole: most decisions come from what is said.
- `view_frames` on each clip you may use: the start, the middle and the end. Note what is in it and what is wrong with it: focus, glare, a hand in frame.
- Write it under Footage in PROJECT.md, so the next chat doesn't look again.

## 2. Plan on paper

Pick the kind of video and read its file with `read_skill` (`name: video-editing`, `file`):

- `talking-head.md`: one person to camera.
- `interview.md`: someone answers questions, with or without the interviewer.
- `scripted.md`: a script or voice-over leads, pictures follow.
- `short-vertical.md`: under about 90 seconds, 9:16.

A video can be two of them, such as a short cut from an interview: read both. For another kind, follow the closest one.

Then write the plan in PROJECT.md before any timeline: every part in order, with its purpose, the clip and the times it comes from, and its length. Add the lengths up and compare the total with the brief. Changing a list costs nothing, changing a built timeline does.

- With a script, take it line by line: which take says it, and which lines no take says. Never put words together that change what someone meant.
- Without one, find the one thing the video is about, then keep what serves it. Cut the rest, also when it is nice.
- The opening earns the rest. Start with the most interesting true thing, not with a greeting.

## 3. Build it

Read `cutting-speech` before you cut anything that is said.

1. `create_timeline` in the brief's size and frame rate. With neither given, take the footage's.
2. Rough cut: the spoken story on V1 and A1, in the plan's order, with `add_clips`. Nothing else yet. Read it back with `get_timeline`.
3. Tighten it until the story works without pictures on top.
4. Cover: B-roll on V2 over jump cuts and where a picture says more than the face. A B-roll shot shows what is being said then, starts on an action, and lasts 2 to 5 seconds.
5. Sound: read `sound` first.
6. Titles: read `titles-and-captions` first.
7. Color: read `color-grading` first, also when the brief doesn't mention color. Shots next to each other should at least match.

Change one thing at a time on a timeline that works, and read it back. When you change a cut after feedback, copy the timeline first (`duplicate_timeline`), and change only what the feedback is about.

## 4. Check it before you show it

- Every line under "Must" in the rules, the length first.
- `get_timeline`: no gaps, nothing on a wrong track, sound where the picture is, music that stops before or with the last picture.
- `view_frames` on the first and last frame, every title, and both sides of a few cuts. No black frames. Titles readable and inside the frame. If a title isn't there, it didn't work.
- The word times at each cut in speech: no cut inside a word.
- You can't hear it or see it move, so measure it: `render_timeline` (a small preset is enough) and `study_media` on the file. Compare length, aspect ratio, frame rate, loudness, the longest quiet stretch and shot lengths with the brief and the rules.

Fix what is off and measure again, at most twice. Then show it and say what is still off.

## 5. The editor's note

Short, in this order:

- What you made: the timeline's name and its length.
- Each "Must" line and each number in the brief, with what you measured.
- What you left out and why, and script lines no take says.
- What you decided instead of asking, one line each, so the user can overrule it.
- What you couldn't check: how it sounds, and motion between frames.

End by asking for feedback in the preview, with times.
