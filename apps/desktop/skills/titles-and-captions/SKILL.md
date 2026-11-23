---
name: titles-and-captions
description: Titles, lower thirds, names and captions on screen. Read it before you add or change any text in the picture.
---

# Titles and captions

Text in the picture competes with the face and the voice. Add it when it says something they don't: a name, a number, the subject, a quote worth reading.

## Words

- As few as work. A lower third is a name and a role, nothing else. A title is up to about six words.
- Spell names and companies as the user, the footage or the script spells them. A name you only heard in a transcript is a guess: mark it in your note.
- Use the brand's words, capitals and language from the rules.

## Making one

- `add_title` with the text, the frame, the font and the size. The rules name the font and the title. With nothing named, use Text+ in a plain font and the same one everywhere.
- It comes on the video track Resolve picks and gets Resolve's standard length, which you can't change. Place it where that length works, and read where it went with `get_timeline`. A title that runs over a cut to another person is wrong.
- Two titles at the same time need two tracks and two places. Better: don't.
- The size is 0 to 0.5, 0.08 is Resolve's default. A phone needs at least that.
- Place a lower third with Pan and Tilt (`set_clip_properties`) on the title's clip.

## Where

- Inside the middle 90% of the picture, and for vertical video inside the middle: the bottom fifth, the top tenth and the right edge are under the phone's buttons.
- Not on a face, not on text in the picture, not on a busy part. Light text on dark, dark on light.
- A name within 10 seconds of the person's first appearance, once they talk, and not on their first word.
- On screen long enough to read twice: about 2 seconds plus a third of a second per word.

## Captions

- Only when the user or the rules ask for them. Never add them because it is a short video.
- When they do: from the transcript, one line of at most about 32 characters (20 in vertical), broken where a person breathes. Check names and numbers against the script.

## Check

`view_frames` on each title at its start, its middle and its end. Is the text there, whole, readable and in the right place? If it isn't there, say so and don't report it as done.
