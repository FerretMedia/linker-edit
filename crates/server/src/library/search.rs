// Finding files in the library: words in the file's name, tags and note, its kind, and a length range.
// All on this computer; nothing is sent anywhere.

use std::sync::LazyLock;

use linker_edit_protocol::{LibraryFile, LibraryQuery, LibraryScopeKind};
use regex::Regex;

use crate::js;

fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(|c: char| c.is_whitespace() || c == ',')
        .filter(|word| !word.is_empty())
        .map(str::to_string)
        .collect()
}

// "Upbeat_Summer-Vibes.mp3" matches "summer vibes".
fn searchable(text: &str) -> String {
    static SEPARATORS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[_\-.]+").unwrap());
    SEPARATORS
        .replace_all(&text.to_lowercase(), " ")
        .into_owned()
}

/// How well a file matches the words: 0 when one is missing, more when they're in its name.
fn score(file: &LibraryFile, query: &[String]) -> u32 {
    if query.is_empty() {
        return 1;
    }
    let name = searchable(&file.name);
    let mut rest = vec![file.rel.as_str()];
    rest.extend(file.tags.iter().map(String::as_str));
    rest.push(&file.note);
    let rest = searchable(&rest.join(" "));
    let mut total = 0;
    for word in query {
        if name.contains(word.as_str()) {
            total += 3;
        } else if rest.contains(word.as_str()) {
            total += 1;
        } else {
            return 0;
        }
    }
    total
}

/// The files that match, best first. scope_order names the scopes least specific first, as the chain goes: files
/// from the most specific library come first, then starred ones, then the best matches.
pub fn search_library(
    files: Vec<LibraryFile>,
    query: &LibraryQuery,
    scope_order: &[LibraryScopeKind],
) -> Vec<LibraryFile> {
    let terms = words(query.query.as_deref().unwrap_or(""));
    let tags: Vec<String> = query
        .tags
        .iter()
        .flatten()
        .map(|tag| js::trim(tag).to_lowercase())
        .filter(|tag| !tag.is_empty())
        .collect();
    let min = query.min_seconds.filter(|value| value.is_finite());
    let max = query.max_seconds.filter(|value| value.is_finite());
    let rank = |file: &LibraryFile| {
        scope_order
            .iter()
            .position(|scope| *scope == file.scope)
            .map_or(-1, |index| index as i64)
    };
    let mut found: Vec<(LibraryFile, u32)> = files
        .into_iter()
        .filter(|file| query.kind.is_none_or(|kind| file.kind == kind))
        .filter(|file| query.starred != Some(true) || file.star)
        .filter(|file| {
            tags.iter()
                .all(|tag| file.tags.iter().any(|own| own.to_lowercase() == *tag))
        })
        .filter(|file| {
            (min.is_none() && max.is_none())
                || file.duration.is_some_and(|duration| {
                    min.is_none_or(|min| duration >= min) && max.is_none_or(|max| duration <= max)
                })
        })
        .map(|file| {
            let score = score(&file, &terms);
            (file, score)
        })
        .filter(|(_, score)| *score > 0)
        .collect();
    found.sort_by(|(a, a_score), (b, b_score)| {
        rank(b)
            .cmp(&rank(a))
            .then_with(|| b.star.cmp(&a.star))
            .then_with(|| b_score.cmp(a_score))
            .then_with(|| js::locale_compare_numeric_base(&a.rel, &b.rel))
    });
    found.into_iter().map(|(file, _)| file).collect()
}

#[cfg(test)]
mod tests {
    use linker_edit_protocol::LibraryKind;

    use super::*;

    fn file(rel: &str, change: impl FnOnce(&mut LibraryFile)) -> LibraryFile {
        let mut file = LibraryFile {
            folder_id: "music".into(),
            scope: LibraryScopeKind::Universal,
            kind: LibraryKind::Music,
            rel: rel.into(),
            name: rel.rsplit('/').next().unwrap().into(),
            path: format!("/library/{rel}"),
            size: 1,
            modified: "2026-11-15T10:00:00.000Z".into(),
            duration: Some(120.0),
            channels: Some(2),
            loudness: Some(-14.0),
            tags: vec![],
            star: false,
            note: String::new(),
            used_in: vec![],
        };
        change(&mut file);
        file
    }

    fn files() -> Vec<LibraryFile> {
        vec![
            file("Upbeat_Summer-Vibes.mp3", |f| {
                f.duration = Some(150.0);
                f.tags = vec!["happy".into()];
            }),
            file("Calm/Piano.wav", |f| {
                f.duration = Some(200.0);
                f.note = "upbeat at the end".into();
            }),
            file("Whoosh 1.wav", |f| {
                f.kind = LibraryKind::Sfx;
                f.folder_id = "sfx".into();
                f.duration = Some(1.2);
                f.tags = vec!["Transition".into()];
            }),
            file("Whoosh 2.wav", |f| {
                f.kind = LibraryKind::Sfx;
                f.folder_id = "sfx".into();
                f.duration = Some(0.8);
                f.tags = vec!["transition".into()];
                f.star = true;
            }),
            file("Logo.png", |f| {
                f.kind = LibraryKind::Graphics;
                f.folder_id = "logos".into();
                f.duration = None;
            }),
            file("Theme.wav", |f| {
                f.scope = LibraryScopeKind::Project;
                f.folder_id = "own".into();
                f.duration = Some(90.0);
            }),
        ]
    }

    fn names(query: LibraryQuery, order: &[LibraryScopeKind]) -> Vec<String> {
        search_library(files(), &query, order)
            .into_iter()
            .map(|file| file.name)
            .collect()
    }

    fn words_query(text: &str) -> LibraryQuery {
        LibraryQuery {
            query: Some(text.into()),
            ..LibraryQuery::default()
        }
    }

    #[test]
    fn finds_words_in_names_first_then_in_tags_and_notes_ignoring_underscores_and_dashes() {
        assert_eq!(
            names(words_query("upbeat"), &[]),
            ["Upbeat_Summer-Vibes.mp3", "Piano.wav"]
        );
        assert_eq!(
            names(words_query("summer vibes"), &[]),
            ["Upbeat_Summer-Vibes.mp3"]
        );
        assert_eq!(
            names(words_query("happy"), &[]),
            ["Upbeat_Summer-Vibes.mp3"]
        );
        assert_eq!(names(words_query("calm"), &[]), ["Piano.wav"]);
        assert!(names(words_query("upbeat nothing"), &[]).is_empty());
    }

    #[test]
    fn filters_by_kind_tags_stars_and_length() {
        let query = |change: fn(&mut LibraryQuery)| {
            let mut query = LibraryQuery::default();
            change(&mut query);
            names(query, &[])
        };
        assert_eq!(
            query(|q| q.kind = Some(LibraryKind::Sfx)),
            ["Whoosh 2.wav", "Whoosh 1.wav"]
        );
        assert_eq!(
            query(|q| q.tags = Some(vec!["transition".into()])),
            ["Whoosh 2.wav", "Whoosh 1.wav"]
        );
        assert!(query(|q| q.tags = Some(vec!["transition".into(), "happy".into()])).is_empty());
        assert_eq!(query(|q| q.starred = Some(true)), ["Whoosh 2.wav"]);
        assert_eq!(
            query(|q| {
                q.min_seconds = Some(1.0);
                q.max_seconds = Some(160.0);
            }),
            ["Theme.wav", "Upbeat_Summer-Vibes.mp3", "Whoosh 1.wav"]
        );
        // A still has no length, so a length range leaves it out.
        assert!(!query(|q| q.max_seconds = Some(10_000.0)).contains(&"Logo.png".to_string()));
    }

    #[test]
    fn puts_the_most_specific_library_first() {
        let music = || LibraryQuery {
            kind: Some(LibraryKind::Music),
            ..LibraryQuery::default()
        };
        use LibraryScopeKind::{Project, Universal};
        assert_eq!(names(music(), &[Universal, Project])[0], "Theme.wav");
        assert_eq!(
            names(music(), &[Project, Universal]).last().unwrap(),
            "Theme.wav"
        );
    }
}
