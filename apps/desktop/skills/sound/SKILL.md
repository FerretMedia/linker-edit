---
name: sound
description: Music, sound effects and levels, such as choosing a song, setting it under speech and measuring loudness. Read it before you add or change any sound.
---

# Sound

You can't hear. You can read what is said, see where it is loud or quiet, and measure. Choose with that, and say what a person has to listen to.

## Choosing music

- `library_search` with `kind: music`: first the words the rules or the brief use, then mood words. The library's tags, notes and stars are the user's ears: trust them over a file name.
- Respect every licensing note. When a folder's note doesn't allow this use, don't use it. Don't take a song that was used in another video of the series, unless the rules say so.
- Long enough for the part it covers. A song that has to be cut short ends under speech or on a picture change.
- Nothing fits? Leave music out and say what you looked for. A wrong song is worse than none.
- With speech, prefer music without singing (tags and notes say so).

## Placing it

- Voice on A1, B-roll's own sound on A2, music on A3, effects on A4. Name the tracks with `set_tracks`.
- Music starts with a reason: the first picture, a new part, after the opening line. It stops or changes where the subject does. Wall to wall music is background noise.
- Silence is a tool: take the music away before the line that matters most.
- Effects on something that happens, such as a cut, a title or an action, at the frame it happens. Few.

## Levels

- `set_clip_properties` with `AudioVolume` in dB, per clip. There are no fades or automation here: start and end music under speech or exactly on a cut, and say where a fade is needed.
- The voice comes first. Music under speech about 18 to 22 dB below the voice, music alone about 8 to 12 dB below. The library gives a song's loudness (LUFS): a song at -10 LUFS needs about 26 dB down to sit 20 dB under a voice at -16.
- Two clips of one speaker at different loudness: bring the quiet one up, at most 6 dB, more makes the noise loud.
- B-roll's own sound at -20 dB or lower under speech. It makes a place feel real. Mute it when there is talk in it.

## Measuring

- `render_timeline` with the picture or `audio_only`, then `study_media`. It gives the loudness of the whole and how quiet it gets.
- Aim for what the rules say. With nothing said: about -14 LUFS for YouTube and social, -23 for broadcast in Europe. Off by more than 2: change all levels by the same amount, at most 6 dB up. Nothing here measures peaks, so say that they need a look before delivery.
- A quiet stretch over a second that you didn't plan is usually a missing clip or a gap.

In your note: the song and where it is from, its license note, the measured loudness, and "please listen to" with times: the start and end of music, and the loudest moment.
