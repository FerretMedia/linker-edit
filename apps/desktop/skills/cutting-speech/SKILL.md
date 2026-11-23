---
name: cutting-speech
description: Cutting what people say, such as retakes, false starts, silences, filler words and long answers. Read it before you cut or tighten any clip with speech in it.
---

# Cutting speech

Work from the transcript's word times, not from the sentence times: the JSON file that `transcribe` names has a start and end for each word.

## What goes

1. **False starts and retakes.** The same words twice, "sorry", "again", "let me say that again", a clap, a sentence that stops. Keep the last complete take, unless an earlier one is clearly better.
2. **Silences** that say nothing. `find_silences` finds them without transcribing.
3. **Filler** ("um", "you know", "like", "eigenlijk") when it stands between two pauses. Filler that runs into the next word stays: cutting it sounds worse than keeping it.
4. **Repeats.** Two sentences with one meaning: keep the clearer one.
5. **Everything off the subject.**

## What stays

- A pause before or after something that matters.
- Breath between sentences. Speech without any breath sounds like a machine.
- Words that limit a claim: "about", "I think", "not always".
- The speaker's way of talking. Tidy it, don't rewrite it.

## Where to cut

- Between words, in the quiet. Start about 0.12 seconds before the first word and end about 0.2 seconds after the last, in frames of the clip. Whisper's word times can be off by a tenth of a second, so a smaller margin clips words.
- Never inside a word. After building, compare every cut's source frames with the word times.
- After a full thought, with the voice going down. A cut in the middle of a rising sentence sounds broken.
- Source frame = seconds × the clip's own frame rate, then round away from the word. A timeline with another frame rate changes timeline frames, not source frames.
- Pauses left in: about 0.2 seconds inside a sentence, 0.4 between sentences, 0.6 or more between subjects. Faster for short vertical, slower for an interview.

## Building it

- One clip per range you keep, back to back, with `add_clips` (`type: both`, so picture and sound stay together).
- Keep ranges in their order, unless the plan moves a whole thought.
- Don't make a range shorter than about 12 frames, except on purpose.
- Read it back with `get_timeline`, and check that the total matches the plan.

In your note, quote what you dropped when it was more than filler, with times, so the user can ask for it back.
