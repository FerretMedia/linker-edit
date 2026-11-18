// Raw footage against the finished video, by what is said: lines the final cut's words up with the raw footage's,
// which shows what the editor kept, what they cut, which take they used and in what order. No project file needed.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use serde::Serialize;

use crate::js;
use crate::transcribe::whisper::Transcript;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub word: String,
    pub start: f64,
    pub end: f64,
}

/// Which take of the words this is, when the raw footage has them more than once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Take {
    pub used: usize,
    pub of: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct KeptPart {
    /// Seconds in the finished video.
    #[serde(serialize_with = "js::nums")]
    pub r#final: [f64; 2],
    /// The raw file it comes from, and where in it, in seconds.
    pub raw_file: String,
    #[serde(serialize_with = "js::nums")]
    pub raw: [f64; 2],
    pub words: usize,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub take: Option<Take>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CutKind {
    /// Most of it is said again in a part that was kept.
    Retake,
    /// It isn't in the finished video at all.
    Content,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CutPart {
    pub raw_file: String,
    #[serde(serialize_with = "js::nums")]
    pub raw: [f64; 2],
    pub words: usize,
    pub text: String,
    pub kind: CutKind,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CutComparison {
    #[serde(serialize_with = "js::num")]
    pub final_seconds: f64,
    #[serde(serialize_with = "js::num")]
    pub raw_seconds: f64,
    pub raw_words: usize,
    /// How much of what was said in the raw footage is in the finished video, 0 to 1.
    #[serde(serialize_with = "js::num")]
    pub kept_share: f64,
    /// How much of what is said in the finished video wasn't found in the raw footage (voice-over, other footage),
    /// 0 to 1.
    #[serde(serialize_with = "js::num")]
    pub not_from_raw_share: f64,
    pub parts: usize,
    #[serde(serialize_with = "js::num")]
    pub average_part_seconds: f64,
    /// The silence left where two parts meet, in the finished video.
    #[serde(serialize_with = "js::num")]
    pub median_gap_at_cuts_seconds: f64,
    /// How often the finished video jumps back in the raw footage, or to another file: the editor reordered.
    pub reordered: usize,
    pub kept: Vec<KeptPart>,
    pub cut: Vec<CutPart>,
    /// More parts than listed.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
}

const GRAM: usize = 3;
const MIN_RUN: usize = 4;
const MAX_LISTED: usize = 60;
// A cut stretch shorter than this is a word or two trimmed, not a decision worth listing.
const MIN_CUT_WORDS: usize = 6;

fn round(value: f64) -> f64 {
    js::round_to(value, 2)
}

/// Words as they are compared: lower case, no punctuation. Words that are only punctuation are dropped.
pub fn tokens(transcript: &Transcript) -> Vec<Token> {
    static NOT_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^\p{L}\p{N}']+").unwrap());
    transcript
        .segments
        .iter()
        .flat_map(|segment| &segment.words)
        .map(|word| Token {
            word: NOT_WORD
                .replace_all(&word.text.to_lowercase(), "")
                .into_owned(),
            start: word.start,
            end: word.end,
        })
        .filter(|token| !token.word.is_empty())
        .collect()
}

fn gram_at(list: &[Token], index: usize) -> String {
    format!(
        "{} {} {}",
        list[index].word,
        list[index + 1].word,
        list[index + 2].word
    )
}

fn snippet(list: &[Token], from: usize, to: usize) -> String {
    let words: Vec<&str> = list[from..to]
        .iter()
        .map(|token| token.word.as_str())
        .collect();
    if words.len() <= 16 {
        words.join(" ")
    } else {
        format!(
            "{} … {}",
            words[..8].join(" "),
            words[words.len() - 6..].join(" ")
        )
    }
}

#[derive(Debug, Clone)]
struct Run {
    final_from: usize,
    final_to: usize,
    file: usize,
    raw_from: usize,
    raw_to: usize,
    /// Every place these words are said, as (file, word), in order.
    takes: Vec<(usize, usize)>,
}

#[derive(Debug, Clone, Copy)]
struct Place {
    file: usize,
    at: usize,
    length: usize,
}

/// Compares the finished video's transcript with the raw footage's.
pub fn compare_cut(r#final: &Transcript, raw: &[Transcript]) -> CutComparison {
    let final_tokens = tokens(r#final);
    let raw_tokens: Vec<Vec<Token>> = raw.iter().map(tokens).collect();

    // Where each run of three words is said in the raw footage.
    let mut index: HashMap<String, Vec<(usize, usize)>> = HashMap::new();
    for (file, list) in raw_tokens.iter().enumerate() {
        for at in 0..(list.len() + 1).saturating_sub(GRAM) {
            index.entry(gram_at(list, at)).or_default().push((file, at));
        }
    }

    let match_length = |final_at: usize, file: usize, raw_at: usize| {
        let list = &raw_tokens[file];
        let same = |offset: usize| {
            final_at + offset < final_tokens.len()
                && raw_at + offset < list.len()
                && final_tokens[final_at + offset].word == list[raw_at + offset].word
        };
        let mut length = 0;
        // One word heard differently in the two transcripts doesn't end a match when the next two agree again.
        while same(length) || (length > 0 && same(length + 1) && same(length + 2)) {
            length += 1;
        }
        length
    };

    let mut runs: Vec<Run> = Vec::new();
    let mut at = 0;
    while at + GRAM <= final_tokens.len() {
        let places = index
            .get(&gram_at(&final_tokens, at))
            .cloned()
            .unwrap_or_default();
        let previous = runs.last();
        let lengths: Vec<Place> = places
            .iter()
            .map(|&(file, raw_at)| Place {
                file,
                at: raw_at,
                length: match_length(at, file, raw_at),
            })
            .collect();
        // The longest match wins; between equals, the one that goes on from where the last part ended.
        let follows = |place: &Place| match previous {
            Some(previous) if place.file == previous.file && place.at >= previous.raw_to => {
                place.at - previous.raw_to
            }
            _ => usize::MAX,
        };
        let mut best: Option<Place> = None;
        for candidate in lengths
            .iter()
            .filter(|candidate| candidate.length >= MIN_RUN)
        {
            let better = match &best {
                None => true,
                Some(best) => {
                    candidate.length > best.length
                        || (candidate.length == best.length && follows(candidate) < follows(best))
                }
            };
            if better {
                best = Some(*candidate);
            }
        }
        let Some(best) = best else {
            at += 1;
            continue;
        };
        // The same words said elsewhere in the raw footage are other takes of this part.
        let length = best.length;
        let mut takes: Vec<(usize, usize)> = lengths
            .iter()
            .filter(|candidate| candidate.length >= length.min(8))
            .map(|candidate| (candidate.file, candidate.at))
            .collect();
        takes.sort();
        runs.push(Run {
            final_from: at,
            final_to: at + length,
            file: best.file,
            raw_from: best.at,
            raw_to: best.at + length,
            takes,
        });
        at += length;
    }

    // A misheard word splits one kept part in two; parts that carry on in the raw footage are one.
    let mut merged: Vec<Run> = Vec::new();
    for run in runs {
        let list = &raw_tokens[run.file];
        let continues = merged.last().is_some_and(|last| {
            last.file == run.file
                && run.raw_from >= last.raw_to
                && run.raw_from - last.raw_to <= 3
                && run.final_from.saturating_sub(last.final_to) <= 3
                && list[run.raw_from].start - list[last.raw_to - 1].end < 1.0
        });
        match merged.last_mut() {
            Some(last) if continues => {
                last.final_to = run.final_to;
                last.raw_to = run.raw_to;
            }
            _ => merged.push(run),
        }
    }

    let kept: Vec<KeptPart> = merged
        .iter()
        .map(|run| {
            let list = &raw_tokens[run.file];
            let used = run
                .takes
                .iter()
                .position(|take| *take == (run.file, run.raw_from))
                .map_or(0, |index| index + 1);
            KeptPart {
                r#final: [
                    round(final_tokens[run.final_from].start),
                    round(final_tokens[run.final_to - 1].end),
                ],
                raw_file: raw[run.file].file.clone(),
                raw: [
                    round(list[run.raw_from].start),
                    round(list[run.raw_to - 1].end),
                ],
                words: run.final_to - run.final_from,
                text: snippet(&final_tokens, run.final_from, run.final_to),
                take: (run.takes.len() > 1 && used > 0).then_some(Take {
                    used,
                    of: run.takes.len(),
                }),
            }
        })
        .collect();

    // What's left of the raw footage, and whether it was said again in what was kept.
    let final_grams: HashSet<String> = (0..(final_tokens.len() + 1).saturating_sub(GRAM))
        .map(|i| gram_at(&final_tokens, i))
        .collect();
    let mut cut: Vec<CutPart> = Vec::new();
    for (file, list) in raw_tokens.iter().enumerate() {
        let mut covered = vec![false; list.len()];
        for run in merged.iter().filter(|run| run.file == file) {
            covered[run.raw_from..run.raw_to].fill(true);
        }
        let mut from = 0;
        while from < list.len() {
            if covered[from] {
                from += 1;
                continue;
            }
            let mut to = from;
            while to < list.len() && !covered[to] {
                to += 1;
            }
            if to - from >= MIN_CUT_WORDS {
                let grams = (to - from) as f64 - GRAM as f64 + 1.0;
                let repeated = (from..=to - GRAM)
                    .filter(|&i| final_grams.contains(&gram_at(list, i)))
                    .count();
                cut.push(CutPart {
                    raw_file: raw[file].file.clone(),
                    raw: [round(list[from].start), round(list[to - 1].end)],
                    words: to - from,
                    text: snippet(list, from, to),
                    kind: if grams > 0.0 && repeated as f64 / grams >= 0.5 {
                        CutKind::Retake
                    } else {
                        CutKind::Content
                    },
                });
            }
            from = to;
        }
    }

    let kept_words: usize = merged.iter().map(|run| run.raw_to - run.raw_from).sum();
    let matched_final: usize = merged.iter().map(|run| run.final_to - run.final_from).sum();
    let raw_words: usize = raw_tokens.iter().map(Vec::len).sum();
    let mut gaps: Vec<f64> = kept
        .windows(2)
        .map(|pair| pair[1].r#final[0] - pair[0].r#final[1])
        .filter(|gap| *gap >= 0.0)
        .collect();
    gaps.sort_by(f64::total_cmp);
    let reordered = merged
        .windows(2)
        .filter(|pair| pair[1].file != pair[0].file || pair[1].raw_from < pair[0].raw_to)
        .count();
    let part_seconds: f64 = kept
        .iter()
        .map(|part| part.r#final[1] - part.r#final[0])
        .sum();
    let truncated = kept.len() > MAX_LISTED || cut.len() > MAX_LISTED;

    CutComparison {
        final_seconds: round(r#final.duration),
        raw_seconds: round(raw.iter().map(|transcript| transcript.duration).sum()),
        raw_words,
        kept_share: if raw_words > 0 {
            round(kept_words as f64 / raw_words as f64)
        } else {
            0.0
        },
        not_from_raw_share: if final_tokens.is_empty() {
            0.0
        } else {
            round(1.0 - matched_final as f64 / final_tokens.len() as f64)
        },
        parts: kept.len(),
        average_part_seconds: if kept.is_empty() {
            0.0
        } else {
            round(part_seconds / kept.len() as f64)
        },
        median_gap_at_cuts_seconds: round(gaps.get(gaps.len() / 2).copied().unwrap_or(0.0)),
        reordered,
        kept: kept.into_iter().take(MAX_LISTED).collect(),
        cut: cut.into_iter().take(MAX_LISTED).collect(),
        truncated,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::path::Path;

    use super::*;
    use crate::transcribe::whisper::{Segment, Word};

    /// A transcript of evenly spoken words; "|" is a two-second pause.
    pub(crate) fn transcript(file: &str, text: &str) -> Transcript {
        let mut at = 0.0;
        let mut words = Vec::new();
        for word in text.split_whitespace() {
            if word == "|" {
                at += 2.0;
                continue;
            }
            words.push(Word {
                start: at,
                end: at + 0.4,
                text: word.into(),
            });
            at += 0.5;
        }
        Transcript {
            file: file.into(),
            size: 1,
            modified: 1.0,
            engine: "test".into(),
            language: "en".into(),
            duration: at,
            segments: vec![Segment {
                start: 0.0,
                end: at,
                text: text.into(),
                words,
            }],
        }
    }

    fn raw() -> Transcript {
        transcript(
            "/shoot/a.mov",
            "Hello, and welcome to the show today we talk about cameras | um let me start again | hello and welcome to the show today we talk about lenses and tripods | this part is boring nobody cares about my cat at all really",
        )
    }

    #[test]
    fn finds_what_was_kept_which_take_it_was_and_what_was_cut_and_why() {
        let result = compare_cut(
            &transcript(
                "/final.mp4",
                "Hello and welcome to the show. Today we talk about lenses and tripods!",
            ),
            &[raw()],
        );
        assert_eq!(
            result.kept,
            [KeptPart {
                r#final: [0.0, 6.4],
                raw_file: "/shoot/a.mov".into(),
                raw: [12.0, 18.4],
                words: 13,
                text: "hello and welcome to the show today we talk about lenses and tripods".into(),
                // Said twice in the raw footage; the editor used the second take.
                take: Some(Take { used: 2, of: 2 }),
            }]
        );
        assert_eq!(
            (
                result.raw_words,
                result.kept_share,
                result.not_from_raw_share,
                result.parts,
                result.reordered
            ),
            (41, 0.32, 0.0, 1, 0)
        );
        let cut: Vec<_> = result
            .cut
            .iter()
            .map(|part| (part.kind, part.words, part.raw))
            .collect();
        assert_eq!(
            cut,
            [
                (CutKind::Retake, 16, [0.0, 9.9]),
                (CutKind::Content, 12, [20.5, 26.4])
            ]
        );
    }

    #[test]
    fn counts_jumps_back_in_the_raw_footage_as_reordering_and_words_from_elsewhere_as_not_from_it()
    {
        let r#final = transcript(
            "/final.mp4",
            "nobody cares about my cat | and now a word from our sponsor | hello and welcome to the show today we talk about lenses",
        );
        let result = compare_cut(&r#final, &[raw()]);
        let texts: Vec<_> = result.kept.iter().map(|part| part.text.as_str()).collect();
        assert_eq!(
            texts,
            [
                "nobody cares about my cat",
                "hello and welcome to the show today we talk about lenses"
            ]
        );
        assert_eq!(
            (result.parts, result.reordered, result.not_from_raw_share),
            (2, 1, 0.3)
        );
        // The silence left where the two parts meet, sponsor read included.
        assert!(result.median_gap_at_cuts_seconds > 2.0);
    }

    #[test]
    fn keeps_apart_raw_files_and_joins_a_part_split_by_one_misheard_word() {
        let b = transcript(
            "/shoot/b.mov",
            "the second camera angle shows the product from the side",
        );
        let r#final = transcript(
            "/final.mp4",
            "hello and welcome to the snow today we talk about lenses the second camera angle shows the product",
        );
        let result = compare_cut(&r#final, &[raw(), b]);
        let parts: Vec<_> = result
            .kept
            .iter()
            .map(|part| {
                let name = Path::new(&part.raw_file).file_name().unwrap();
                (name.to_string_lossy().into_owned(), part.words)
            })
            .collect();
        assert_eq!(parts, [("a.mov".to_string(), 11), ("b.mov".to_string(), 7)]);
        assert_eq!(result.reordered, 1);
    }
}
