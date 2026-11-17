// Fixtures are Resolve exports from davinci-resolve-mcp (MIT, see NOTICE): Resolve 19.1.3 (format 14) and
// 21.0.4 (format 17), a .drp with a sub-bin, one with compound clips and a nested timeline, and a .drt.

use std::sync::LazyLock;

use linker_edit_drp::blobs::{decode_in, decode_sized, encode_in, from_hex, read_proto, to_hex};
use linker_edit_drp::zip::{entry_data, read_zip};
use linker_edit_drp::{
    DrpFile, Edit, Timeline, TimelineClip, Track, TrackType, VersionCheck, check_version,
    media_items, patch_drp, read_drp,
};
use regex::Regex;

const BINS: &[u8] = include_bytes!("fixtures/bins-r19.drp");
const COMPOUND: &[u8] = include_bytes!("fixtures/compound-r21.drp");
const NESTED: &[u8] = include_bytes!("fixtures/nested-r19.drt");

fn read(buffer: &[u8]) -> DrpFile {
    read_drp(buffer).unwrap()
}

fn timeline<'a>(file: &'a DrpFile, name: &str) -> &'a Timeline {
    file.timelines
        .iter()
        .find(|candidate| candidate.name == name)
        .unwrap_or_else(|| panic!("No timeline {name}"))
}

fn track<'a>(file: &'a DrpFile, timeline_name: &str, label: &str) -> &'a Track {
    let kind = if label.starts_with('A') {
        TrackType::Audio
    } else {
        TrackType::Video
    };
    let index: usize = label[1..].parse().unwrap();
    timeline(file, timeline_name)
        .tracks
        .iter()
        .find(|candidate| candidate.kind == kind && candidate.index == index)
        .unwrap_or_else(|| panic!("No track {label}"))
}

fn clip<'a>(file: &'a DrpFile, timeline_name: &str, label: &str, name: &str) -> &'a TimelineClip {
    track(file, timeline_name, label)
        .clips
        .iter()
        .find(|candidate| candidate.name == name)
        .unwrap_or_else(|| panic!("No clip {name}"))
}

struct Patched {
    before: DrpFile,
    out: Vec<u8>,
    after: DrpFile,
}

fn patched(buffer: &[u8], edits: impl FnOnce(&DrpFile) -> Vec<Edit>) -> Patched {
    let before = read(buffer);
    let out = patch_drp(&before, &edits(&before)).unwrap();
    let after = read(&out);
    Patched { before, out, after }
}

/// The error `patch_drp` gives, which must match `pattern`.
fn refused(file: &DrpFile, edits: &[Edit], pattern: &str) {
    let error = patch_drp(file, edits)
        .expect_err("the edit should be refused")
        .to_string();
    assert!(
        Regex::new(pattern).unwrap().is_match(&error),
        "{error:?} doesn't match /{pattern}/"
    );
}

fn move_clip(clip_id: &str, start: i64, track_id: Option<&str>) -> Edit {
    Edit::MoveClip {
        clip_id: clip_id.into(),
        start,
        track_id: track_id.map(Into::into),
    }
}

fn trim_clip(clip_id: &str, start: Option<i64>, end: Option<i64>) -> Edit {
    Edit::TrimClip {
        clip_id: clip_id.into(),
        start,
        end,
    }
}

fn starts(track: &Track) -> Vec<(&str, i64)> {
    track
        .clips
        .iter()
        .map(|clip| (clip.name.as_str(), clip.start))
        .collect()
}

static CLIP_BLOB: LazyLock<Regex> =
    LazyLock::new(|| Regex::new("<Clip>([0-9a-f]+)</Clip>").unwrap());

fn clip_blobs(file: &DrpFile) -> Vec<String> {
    file.xml
        .values()
        .flat_map(|text| {
            CLIP_BLOB
                .captures_iter(text)
                .map(|captures| captures[1].to_string())
                .collect::<Vec<_>>()
        })
        .collect()
}

// Reading

#[test]
fn reads_a_resolve_19_project_version_media_pool_bins_media_and_a_timeline() {
    let file = read(BINS);
    assert_eq!(file.project.name, "_mcp_e89_tpl16");
    assert_eq!(file.project.db_app_ver.as_deref(), Some("19.1.3.0007"));
    assert_eq!(file.project.db_prj_ver, Some(14));

    assert_eq!(
        file.media_pool
            .iter()
            .map(|folder| folder.name.as_str())
            .collect::<Vec<_>>(),
        ["Master"]
    );
    assert_eq!(
        file.media_pool[0]
            .folders
            .iter()
            .map(|folder| folder.name.as_str())
            .collect::<Vec<_>>(),
        ["Archive"]
    );
    let media = media_items(&file.media_pool);
    let video = media.iter().find(|item| item.kind == "video").unwrap();
    assert_eq!(video.name, "r19_template_media.mp4");
    assert_eq!(video.path.as_deref(), Some("/tmp/r19_template_media.mp4"));
    assert_eq!(
        (video.width, video.height, video.frames),
        (Some(352), Some(262), Some(4576))
    );
    assert!((video.fps.unwrap() - 29.97).abs() < 0.005);

    let edit = timeline(&file, "MediaTemplate");
    assert_eq!(edit.kind, "timeline");
    assert_eq!(edit.fps, Some(24.0));
    assert_eq!((edit.width, edit.height), (Some(1920), Some(1080)));
    assert_eq!((edit.start, edit.frames), (Some(86400), Some(3664)));
    assert_eq!(
        edit.tracks
            .iter()
            .filter(|candidate| candidate.kind == TrackType::Video)
            .count(),
        1
    );
    assert_eq!(
        edit.tracks
            .iter()
            .filter(|candidate| candidate.kind == TrackType::Audio)
            .count(),
        16
    );
    let first = clip(&file, "MediaTemplate", "V1", "r19_template_media.mp4");
    assert_eq!(first.kind, "video");
    assert_eq!((first.start, first.end), (86400, 90064));
    assert_eq!((first.source_in, first.source_out), (Some(0), Some(3664)));
    assert_eq!(first.media_id.as_deref(), Some(video.id.as_str()));
    assert_eq!(
        first.media_path.as_deref(),
        Some("/tmp/r19_template_media.mp4")
    );
    assert_eq!(first.enabled, None);
}

#[test]
fn reads_compound_clips_and_nested_timelines_as_timelines_of_their_own() {
    let file = read(COMPOUND);
    assert_eq!(file.project.name, "ZZ_COMPOUND_RE");
    assert_eq!(file.project.db_app_ver.as_deref(), Some("21.0.4.0005"));
    assert_eq!(file.project.db_prj_ver, Some(17));
    let mut kinds: Vec<_> = file
        .timelines
        .iter()
        .map(|candidate| (candidate.name.as_str(), candidate.kind.as_str()))
        .collect();
    kinds.sort();
    assert_eq!(
        kinds,
        [
            ("CMP_MEDIA", "compound"),
            ("CMP_TITLE", "compound"),
            ("ZZ_CMP_TL", "timeline"),
            ("ZZ_INNER_TL", "timeline")
        ]
    );
    let media = timeline(&file, "CMP_MEDIA");
    assert_eq!((media.start, media.frames), (Some(0), Some(48)));
    let v1: Vec<_> = track(&file, "ZZ_CMP_TL", "V1")
        .clips
        .iter()
        .map(|candidate| (candidate.name.as_str(), candidate.start, candidate.end))
        .collect();
    assert_eq!(
        v1,
        [("CMP_MEDIA", 86400, 86448), ("CMP_TITLE", 86448, 86568)]
    );
    // A clip of a compound clip or timeline plays that media pool item.
    assert_eq!(
        clip(&file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL")
            .media_id
            .as_deref(),
        Some(timeline(&file, "ZZ_INNER_TL").id.as_str())
    );
}

#[test]
fn reads_a_drt_the_same_way_source_in_points_included() {
    let file = read(NESTED);
    assert_eq!(file.project.name, "E57_NESTED");
    assert_eq!(file.project.db_prj_ver, Some(14));
    let mut names: Vec<_> = file
        .timelines
        .iter()
        .map(|candidate| candidate.name.as_str())
        .collect();
    names.sort();
    assert_eq!(names, ["E57_IN", "E57_NESTED", "E57_OUT"]);
    let out = clip(&file, "E57_OUT", "V1", "cut_src.mp4");
    assert_eq!(
        (out.start, out.end, out.source_in, out.source_out),
        (24, 48, Some(96), Some(120))
    );
    let nested = clip(&file, "E57_NESTED", "V1", "cut_src.mp4");
    assert_eq!(nested.source_in, Some(24));
    assert_eq!(
        nested.media_path.as_deref(),
        Some("/Users/samuelgursky/Movies/mcp_probe_tmp/cut_src.mp4")
    );
}

#[test]
fn encodes_in_the_way_resolve_writes_it() {
    // Both values are from the .drt fixture.
    assert_eq!(encode_in(96), "96|fa7e6abc7493b83f");
    assert_eq!(encode_in(24), "24|fa7e6abc7493983f");
    assert_eq!(encode_in(0), "");
    assert_eq!(decode_in("96|fa7e6abc7493b83f"), Ok(96));
}

#[test]
fn refuses_what_isnt_a_drp_or_drt() {
    let error = read_drp(b"not a zip at all, just text")
        .unwrap_err()
        .to_string();
    assert!(error.contains("isn't a ZIP"), "{error}");
}

// Writing

#[test]
fn writes_an_unedited_file_back_byte_for_byte() {
    for buffer in [BINS, COMPOUND, NESTED] {
        assert!(patch_drp(&read(buffer), &[]).unwrap() == buffer);
    }
}

#[test]
fn copies_every_entry_an_edit_doesnt_touch_as_it_was_compressed_data_included() {
    let Patched { out, .. } = patched(COMPOUND, |file| {
        vec![move_clip(
            &clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").id,
            86700,
            None,
        )]
    });
    let before = read_zip(COMPOUND).unwrap().entries;
    let after = read_zip(&out).unwrap().entries;
    assert_eq!(
        after.iter().map(|entry| &entry.name).collect::<Vec<_>>(),
        before.iter().map(|entry| &entry.name).collect::<Vec<_>>()
    );
    let changed: Vec<_> = after
        .iter()
        .zip(&before)
        .filter(|(after, before)| after.local != before.local)
        .map(|(entry, _)| &entry.name)
        .collect();
    assert_eq!(changed, [&timeline(&read(COMPOUND), "ZZ_CMP_TL").entry]);
    // Every entry still passes its CRC check.
    for entry in &after {
        entry_data(entry).unwrap();
    }
}

// Patching

#[test]
fn moves_a_clip_in_time_changing_only_its_start() {
    let Patched { before, after, .. } = patched(COMPOUND, |file| {
        vec![move_clip(
            &clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").id,
            86700,
            None,
        )]
    });
    let moved = clip(&after, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL");
    assert_eq!(
        (moved.start, moved.end, moved.source_in),
        (86700, 86748, Some(0))
    );
    let entry = &timeline(&before, "ZZ_CMP_TL").entry;
    let old_text = &before.xml[entry];
    assert_eq!(
        after.xml[entry],
        old_text.replacen("<Start>86600</Start>", "<Start>86700</Start>", 1)
    );
}

#[test]
fn moves_a_clip_to_another_track_keeping_each_track_in_record_order() {
    let Patched { after, .. } = patched(COMPOUND, |file| {
        let v1 = &track(file, "ZZ_CMP_TL", "V1").id;
        vec![move_clip(
            &clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").id,
            86568,
            Some(v1),
        )]
    });
    assert_eq!(
        starts(track(&after, "ZZ_CMP_TL", "V1")),
        [
            ("CMP_MEDIA", 86400),
            ("CMP_TITLE", 86448),
            ("ZZ_INNER_TL", 86568)
        ]
    );
    assert!(track(&after, "ZZ_CMP_TL", "V2").clips.is_empty());
    assert!(after.xml[&timeline(&after, "ZZ_CMP_TL").entry].contains("<Items/>"));

    // Moving it back, into the now empty track, gives back the XML Resolve wrote.
    let id = &clip(&after, "ZZ_CMP_TL", "V1", "ZZ_INNER_TL").id;
    let v2 = &track(&after, "ZZ_CMP_TL", "V2").id;
    let back = read(&patch_drp(&after, &[move_clip(id, 86600, Some(v2))]).unwrap());
    assert_eq!(back.xml, read(COMPOUND).xml);
}

#[test]
fn reorders_clips_on_their_own_track_when_a_move_passes_another() {
    let Patched { after, .. } = patched(COMPOUND, |file| {
        vec![move_clip(
            &clip(file, "ZZ_CMP_TL", "V1", "CMP_MEDIA").id,
            86568,
            None,
        )]
    });
    assert_eq!(
        starts(track(&after, "ZZ_CMP_TL", "V1")),
        [("CMP_TITLE", 86448), ("CMP_MEDIA", 86568)]
    );
}

#[test]
fn refuses_moves_that_overlap_another_clip_or_change_track_type() {
    let file = read(COMPOUND);
    let media = &clip(&file, "ZZ_CMP_TL", "V1", "CMP_MEDIA").id;
    refused(
        &file,
        &[move_clip(media, 86440, None)],
        r#"overlap "CMP_MEDIA" on V1"#,
    );
    refused(
        &file,
        &[move_clip(
            media,
            0,
            Some(&track(&file, "ZZ_CMP_TL", "A1").id),
        )],
        "own type",
    );
    refused(&file, &[move_clip("nope", 0, None)], "no clip nope");
}

#[test]
fn trims_a_clips_head_and_tail_moving_its_source_in_with_the_head() {
    let Patched { after, .. } = patched(NESTED, |file| {
        vec![trim_clip(
            &clip(file, "E57_OUT", "V1", "cut_src.mp4").id,
            Some(30),
            Some(40),
        )]
    });
    let trimmed = clip(&after, "E57_OUT", "V1", "cut_src.mp4");
    assert_eq!(
        (
            trimmed.start,
            trimmed.end,
            trimmed.source_in,
            trimmed.source_out
        ),
        (30, 40, Some(102), Some(112))
    );
    assert!(
        after.xml[&timeline(&after, "E57_OUT").entry]
            .contains(&format!("<In>{}</In>", encode_in(102)))
    );
}

#[test]
fn refuses_trims_before_the_start_or_past_the_end_of_the_media() {
    let nested = read(NESTED);
    let early = &clip(&nested, "E57_NESTED", "V1", "cut_src.mp4").id;
    refused(
        &nested,
        &[trim_clip(early, Some(86370), None)],
        r"6 frame\(s\) before",
    );
    // 4576 frames at 29.97 fps last 3664 frames at 24 fps, all of which the clip already plays.
    let bins = read(BINS);
    let all = &clip(&bins, "MediaTemplate", "V1", "r19_template_media.mp4").id;
    refused(&bins, &[trim_clip(all, None, Some(90065))], "past the end");
    refused(
        &bins,
        &[trim_clip(all, Some(90064), None)],
        "at least one frame",
    );
}

#[test]
fn removes_a_clip_and_nothing_else() {
    let Patched { before, after, .. } = patched(NESTED, |file| {
        vec![Edit::RemoveClip {
            clip_id: clip(file, "E57_NESTED", "V1", "E57_OUT").id.clone(),
        }]
    });
    let names: Vec<_> = track(&after, "E57_NESTED", "V1")
        .clips
        .iter()
        .map(|candidate| candidate.name.as_str())
        .collect();
    assert_eq!(names, ["cut_src.mp4"]);
    assert_eq!(
        track(&after, "E57_NESTED", "A1").clips,
        track(&before, "E57_NESTED", "A1").clips
    );
    assert!(
        !after.xml[&timeline(&after, "E57_NESTED").entry]
            .contains(&clip(&before, "E57_NESTED", "V1", "E57_OUT").id)
    );
}

#[test]
fn applies_several_edits_to_one_clip_in_order() {
    let Patched { after, .. } = patched(COMPOUND, |file| {
        let id = &clip(file, "ZZ_CMP_TL", "V2", "ZZ_INNER_TL").id;
        vec![
            move_clip(id, 86568, Some(&track(file, "ZZ_CMP_TL", "V1").id)),
            trim_clip(id, None, Some(86590)),
        ]
    });
    let moved = clip(&after, "ZZ_CMP_TL", "V1", "ZZ_INNER_TL");
    assert_eq!((moved.start, moved.end), (86568, 86590));
}

#[test]
fn renames_a_timeline_in_the_media_pool_and_its_sm2timeline() {
    let rename = |file: &DrpFile, name: &str| Edit::RenameTimeline {
        timeline_id: timeline(file, "ZZ_CMP_TL").id.clone(),
        name: name.into(),
    };
    let Patched { before, after, .. } = patched(COMPOUND, |file| vec![rename(file, "Cut <A> & B")]);
    assert!(
        after
            .timelines
            .iter()
            .any(|candidate| candidate.name == "Cut <A> & B")
    );
    let pool = after
        .xml
        .iter()
        .find(|(entry, _)| entry.starts_with("MediaPool/"))
        .unwrap()
        .1;
    assert_eq!(pool.split("<Name>Cut &lt;A&gt; &amp; B</Name>").count(), 3);
    refused(
        &before,
        &[rename(&before, "ZZ_INNER_TL")],
        "already a timeline",
    );
    let video = &media_items(&before.media_pool)
        .into_iter()
        .find(|item| item.kind == "video")
        .unwrap()
        .id;
    refused(
        &before,
        &[Edit::RenameTimeline {
            timeline_id: video.clone(),
            name: "x".into(),
        }],
        "isn't a timeline",
    );
}

#[test]
fn relinks_media_by_rewriting_its_clip_blobs_keeping_their_other_fields() {
    let path = r"D:\Proxies\cut_src.mov";
    let Patched { before, after, .. } = patched(NESTED, |file| {
        let media_id = clip(file, "E57_OUT", "V1", "cut_src.mp4")
            .media_id
            .clone()
            .unwrap();
        vec![Edit::RelinkMedia {
            media_id,
            path: path.into(),
        }]
    });
    let media = |file: &DrpFile, name: &str| {
        media_items(&file.media_pool)
            .into_iter()
            .find(|item| item.name == name)
            .unwrap()
            .path
            .clone()
    };
    assert_eq!(media(&after, "cut_src.mp4").as_deref(), Some(path));
    assert_eq!(
        clip(&after, "E57_OUT", "V1", "cut_src.mp4")
            .media_path
            .as_deref(),
        Some(path)
    );
    assert_eq!(
        clip(&after, "E57_NESTED", "V1", "cut_src.mp4")
            .media_path
            .as_deref(),
        Some(path)
    );
    // The other media keeps its path.
    assert_eq!(
        media(&after, "white_src.mp4").as_deref(),
        Some("/Users/samuelgursky/Movies/mcp_probe_tmp/white_src.mp4")
    );

    let named = |file: &DrpFile, name: &str| -> Vec<String> {
        clip_blobs(file)
            .into_iter()
            .filter(|hex| {
                read_proto(&decode_sized(hex).unwrap().data).unwrap()[1]
                    .payload
                    .as_deref()
                    == Some(name.as_bytes())
            })
            .collect()
    };
    let old = named(&before, "cut_src.mp4");
    let relinked = named(&after, "cut_src.mov");
    assert_eq!(relinked.len(), old.len());
    assert_eq!(old.len(), 2); // video and embedded audio
    for (hex, old) in relinked.iter().zip(&old) {
        let bytes = from_hex(hex);
        // Version 2, a length that counts the marker and the zstd frame, then marker 0x81 and the frame's magic.
        assert_eq!(u32::from_be_bytes(bytes[0..4].try_into().unwrap()), 2);
        assert_eq!(
            u32::from_be_bytes(bytes[4..8].try_into().unwrap()) as usize,
            bytes.len() - 8
        );
        assert_eq!(to_hex(&bytes[8..13]), "8128b52ffd");
        let fields = read_proto(&decode_sized(hex).unwrap().data).unwrap();
        let old_fields = read_proto(&decode_sized(old).unwrap().data).unwrap();
        assert_eq!(
            fields.iter().map(|field| field.field).collect::<Vec<_>>(),
            old_fields
                .iter()
                .map(|field| field.field)
                .collect::<Vec<_>>()
        );
        assert_eq!(fields[0].payload.as_deref(), Some(r"D:\Proxies".as_bytes()));
        assert_eq!(
            fields[2..]
                .iter()
                .map(|field| &field.bytes)
                .collect::<Vec<_>>(),
            old_fields[2..]
                .iter()
                .map(|field| &field.bytes)
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn relinks_back_to_the_original_path_and_gets_the_same_blob_contents() {
    let file = read(BINS);
    let id = media_items(&file.media_pool)
        .into_iter()
        .find(|item| item.kind == "video")
        .unwrap()
        .id
        .clone();
    let relink = |path: &str| Edit::RelinkMedia {
        media_id: id.clone(),
        path: path.into(),
    };
    let away = read(&patch_drp(&file, &[relink("/Volumes/Proxies/a.mov")]).unwrap());
    let back = read(&patch_drp(&away, &[relink("/tmp/r19_template_media.mp4")]).unwrap());
    let payloads = |drp: &DrpFile| -> Vec<Vec<u8>> {
        clip_blobs(drp)
            .iter()
            .map(|hex| decode_sized(hex).unwrap().data)
            .collect()
    };
    assert_ne!(payloads(&away), payloads(&file));
    assert_eq!(payloads(&back), payloads(&file));
    let timeline_id = timeline(&file, "MediaTemplate").id.clone();
    refused(
        &file,
        &[Edit::RelinkMedia {
            media_id: timeline_id,
            path: "/a.mov".into(),
        }],
        "no file to relink",
    );
}

#[test]
fn edits_read_and_write_the_ts_shape() {
    // The window and the iPad send edits as JSON, shaped like the TS `Edit` union.
    let edit: Edit =
        serde_json::from_str(r#"{ "type": "moveClip", "clipId": "c", "start": 86500 }"#).unwrap();
    assert_eq!(edit, move_clip("c", 86500, None));
    let trim = trim_clip("c", None, Some(10));
    assert_eq!(
        serde_json::to_string(&trim).unwrap(),
        r#"{"type":"trimClip","clipId":"c","end":10}"#
    );
}

// checkVersion

#[test]
fn passes_a_file_for_the_same_or_a_newer_resolve() {
    let ok = VersionCheck {
        ok: true,
        message: None,
    };
    assert_eq!(check_version(&read(COMPOUND), 17), ok);
    assert_eq!(check_version(&read(BINS), 17), ok);
    assert_eq!(check_version(&read(NESTED), 14), ok);
}

#[test]
fn explains_why_an_older_resolve_cant_open_a_newer_file() {
    // The message names the Resolve that wrote the file, so the user knows what to update to.
    let check = check_version(&read(COMPOUND), 14);
    assert!(!check.ok);
    assert!(check.message.unwrap().contains("21.0.4 or later"));
}

#[test]
fn refuses_a_file_whose_entries_disagree() {
    let mut file = read(COMPOUND);
    let gallery = file.xml["Gallery.xml"].replacen(r#"DbPrjVer="17""#, r#"DbPrjVer="14""#, 1);
    file.xml.insert("Gallery.xml".into(), gallery);
    let check = check_version(&file, 17);
    assert!(!check.ok);
    assert!(
        check
            .message
            .unwrap()
            .contains("mixes project formats 14 and 17")
    );
}
