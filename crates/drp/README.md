# linker-edit-drp

Reads and patches DaVinci Resolve project (`.drp`) and timeline (`.drt`) files without Resolve: ZIP, zstd and
CRC with `flate2`, `zstd` and `crc32fast`.

It's for two things that come later: browsing any project's timelines from its `.drp` backup without opening it
in Resolve, which has only one project open at a time, and building "Send to iPad" bundles for DaVinci Resolve
for iPad, which has no scripting.

> **Status: nothing written by this library has been opened in Resolve or on the iPad yet.** The reader and
> patcher are tested against Resolve's own exports (see [What's verified](#whats-verified)), not against Resolve
> importing the result. Use a patched file on a copy, and check it in Resolve before relying on it.

## API

```rust
use linker_edit_drp::{Edit, check_version, media_items, patch_drp, read_drp};

let file = read_drp(&std::fs::read("Edit.drp")?)?; // a .drt reads the same way

file.project; // name, db_app_ver: "21.0.4.0005", db_prj_ver: 17
file.media_pool; // bins: id, name, folders, items; items with kind, name, path, fps, width, height, frames
file.timelines; // timelines and compound clips, each with tracks of clips:
// id, kind, name, start, end, source_in, source_out, media_id, media_path, media_fps, enabled

let check = check_version(&file, 14); // whether a Resolve that reads format 14 opens it, and if not why

let patched: Vec<u8> = patch_drp(&file, &[
    Edit::MoveClip { clip_id, start: 86500, track_id: None }, // optionally a track of the same timeline and type
    Edit::TrimClip { clip_id, start: Some(86510), end: Some(86600) }, // record in/out; the source in moves with the in
    Edit::RemoveClip { clip_id },
    Edit::RenameTimeline { timeline_id, name: "Cut 2".into() },
    Edit::RelinkMedia { media_id, path: r"D:\Proxies\A001.mov".into() },
])?;
```

All frame numbers are timeline frames; `end` and `source_out` are exclusive. Edits apply in order. `patch_drp`
returns an error, and nothing else, when an edit names a clip, track or item that isn't there, would make clips overlap
on a track, would run a clip before the start or past the end of its media, or would give two timelines the
same name. It doesn't move linked clips together: move a clip's video and audio with one edit each.

What the patcher keeps:

- Each edit splices the few XML elements it changes into Resolve's text. Files are never re-serialized, and
  blobs other than the `<Clip>` blobs a relink rewrites are left as they are.
- ZIP entries no edit touches are copied byte for byte, compressed data included, in their order; changed ones
  are recompressed with new CRCs. An unedited file writes back identical.

## Format map

A `.drp` and a `.drt` are both a plain ZIP of XML (a `.drt` is a project with the timeline and its media):

| Entry | What's in it |
| --- | --- |
| `project.xml` | `SM_Project`: `ProjectName`, `TimelineHandleVec`, settings |
| `MediaPool/Master/MpFolder.xml` | The Master bin, `Sm2MpFolder` → `MediaVec` → `Sm2MpVideoClip`, `Sm2MpTimelineClip`, `Sm2MpCompoundClip` |
| `MediaPool/Master/<bin>/MpFolder.xml` | One per bin; `<MpFolder>` names the parent bin's DbId |
| `SeqContainer/<uuid>.xml` | One per timeline or compound clip: `Sm2SequenceContainer` → `VideoTrackVec` / `AudioTrackVec` → `Sm2TiTrack` |
| `Gallery.xml` | Stills |

- **Version comment.** Every XML entry starts with `<!--DbAppVer="21.0.4.0005" DbPrjVer="17"-->`. Resolve 21
  writes project format 17, 19.1.3 writes 14. An older Resolve refuses a newer format, which `check_version`
  explains.
- **Objects** are `<Type DbId="uuid">` and refer to each other by DbId. Broken links break the import; there are
  no checksums beyond the ZIP's CRC32.
- **Media pool items.** A timeline or compound clip holds an `Sm2Sequence` (frame rate, resolution, extents);
  a timeline also has an `Sm2Timeline` that repeats its `<Name>`. A media file holds `BtVideoInfo` and
  `BtAudioInfo`, each with a `<Clip>` blob, and `<Time>` / `<Geometry>` keyed dictionaries with frame count,
  frame rate and resolution.
- **Tracks.** `<Type>` is 0 for video, 1 for audio, 2 for subtitles. `<Sequence>` is the media pool item's
  `Sm2Sequence` DbId, which is how a SeqContainer finds its timeline. Clips sit in `<Items>`, one per
  `<Element>`, in record order; an empty track is `<Items/>`.
- **Clips** are `Sm2TiVideoClip` / `Sm2TiAudioClip` with `Name`, `Start` (record frame; 86400 is 01:00:00:00 at
  24 fps), `Duration`, `In`, `MediaRef` (the media pool item), `MediaFilePath` and `MediaFrameRate`.
  - `In` is the source in point in timeline frames, written `frames|<hex little-endian double of frames / 1000>`,
    like `96|fa7e6abc7493b83f`, or empty for 0.
  - `MediaFrameRate`, `FrameRate`: a hex little-endian double and 8 zero bytes. `Resolution`: width and height as
    big-endian u32s at bytes 4 and 12. `MediaExtents`: start and length in seconds, little-endian doubles.
- **Relinking.** Resolve links media by the media pool's `<Clip>` blob, not by `MediaFilePath`, and doesn't
  re-probe files on import, so the stored frame rate, size and length have to match the new file.
  - `<Clip>` is a sized blob: `<u32 version 2> <u32 length> 81 <zstd frame>`, big-endian, where the length counts
    the `81` marker and the frame (`80` instead of `81` means an uncompressed payload).
  - Inside is a protobuf: field 1 the folder (no trailing separator), 2 the file name, 3 a date, 5 the codec,
    6 the file name again (its purpose unknown), 7 a UUID, 13 a timestamp. A relink rewrites fields 1 and 2
    only, plus the clips' `MediaFilePath`.
- **Other blobs.** `FieldsBlob` v1 (`00000001 <count> …`) is a keyed dictionary of UTF-16BE keys and typed values;
  v2 is a sized blob like `<Clip>`. `RenderCacheBA` is `<u32 length> <zlib>`. Effects, grades and markers are
  protobuf. The patcher leaves all of them alone.

## What's verified

Verified, by the tests in `tests/` against three Resolve exports (Resolve 19.1.3 and 21.0.4; see `NOTICE`):

- Reading the project, versions, bins, media (paths, frame rate, size, length), timelines, compound clips,
  nested timelines, tracks and clips, from a `.drp` and a `.drt`.
- Writing an unedited file back byte for byte, and copying untouched entries byte for byte after an edit.
- Each edit re-reads to the expected values; moving a clip away and back gives back Resolve's exact XML.
- `<In>` encodes to the exact bytes Resolve wrote; a relinked `<Clip>` blob keeps every other field and relinking
  back restores the original payload.
- The zstd frames this library writes have the same header shape as Resolve's (content size, no checksum).

Not verified yet:

- **That Resolve or Resolve for iPad opens a patched file.** Nothing written here has been imported anywhere.
  The plan's Phase 0 does this first, with a hand-patched `.drp` on the iPad.
- **Where a disabled clip is stored.** No sample has one, so `enabled` is always `None`.
- **Retimed clips and mixed frame rates.** `In` is read as timeline frames, from the prior art; a trim of a
  retimed clip may be refused when it shouldn't be.
- **Formats other than 14 and 17**, Windows-written files, Fusion and Fairlight data after edits, and bins
  deeper than one level.
