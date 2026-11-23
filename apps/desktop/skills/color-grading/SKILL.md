---
name: color-grading
description: Correcting, matching and grading shots, and using LUTs. Read it before you use set_color, and in every first cut, since shots that don't match look like a mistake.
---

# Color grading

Correct first, match second, look last. A look on shots that don't match makes the difference bigger.

You have `set_color`: a LUT, a CDL (slope, offset, power, saturation), or the grade of another clip copied. There are no curves, qualifiers, windows or tracking, so one clip gets one treatment. Say so when a shot needs more, and name it.

## 1. Look

- `view_frames` on the timeline, one frame in the middle of each shot, the neighbours in one call. You see stills through Resolve's color management, not scopes. Judge differences between shots, which you see well, before absolute values, which you don't.
- Is the footage log? It looks grey, flat and unsaturated. Check the clip's properties (`get_clip_properties`) for its color space. Is color managed by the project already? Then no conversion LUT, or it is converted twice.
- Has the user graded it? A copy of their timeline keeps its grades. Don't touch those unless asked.

## 2. Correct

Set one hero shot per scene right, small steps, then look again:

- **Exposure** with slope (brighter parts) and power (middle). Skin in the middle of the range, no detail lost in white or black. Power below 1 is brighter.
- **White balance** with slope per channel: a warm cast needs less red and more blue, a green one less green. White and grey things should look white and grey. Steps of 0.02 to 0.05.
- **Contrast** comes from slope and offset together. A little less offset gives black back.
- **Saturation** 0.9 to 1.15. More than that looks cheap on skin.

Log footage gets its maker's conversion LUT first (Resolve's `list_luts`, or the library's LUTs), on node 1. With a LUT there, a CDL on the same node changes the picture before the LUT.

## 3. Match

- Same scene, same camera: `copy_from` the hero shot to the others.
- Then look at neighbours in pairs. Match skin first, then white, then black. Change the ones that are off, not the hero.
- Two cameras: match the weaker one to the stronger.

## 4. Look

- Only when the brief, the brand or the series asks for one, or the user does. Otherwise corrected and matched is done.
- A look LUT from the library or the brand at the place the rules name. Skin still has to look like skin.
- Apply it to all shots of the video with the same values.

## 5. Check

- `view_frames` on both sides of every cut between scenes, and on the first and last frame.
- A title on top still reads.
- In your note: what you changed, per scene, in plain words, and which shots need a human eye.
