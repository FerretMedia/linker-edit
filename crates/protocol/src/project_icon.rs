// Project icons as T3 Code has them (apps/web/src/projectIdentity.ts, packages/contracts/src/project.ts; MIT, see NOTICE).

use std::sync::LazyLock;

use regex::Regex;
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use unicode_normalization::UnicodeNormalization;
use unicode_segmentation::UnicodeSegmentation;

use crate::{PROJECT_ICON_COLORS, ProjectIcon, ProjectIconColor};

static LUCIDE_NAME: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-z0-9]+(?:-[a-z0-9]+)*$").unwrap());
static MONOGRAM: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[\p{L}\p{N}][\p{L}\p{N}\p{M}\x{200C}\x{200D}]*$").unwrap());
static WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[\p{L}\p{N}]+").unwrap());
static FLAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\p{Regional_Indicator}{2}$").unwrap());
static KEYCAP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[#*0-9]\x{FE0F}?\x{20E3}$").unwrap());
static PICTOGRAPHIC: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\p{Extended_Pictographic}").unwrap());

fn color(value: &Value) -> Option<ProjectIconColor> {
    serde_json::from_value(value.clone()).ok()
}

/// JavaScript's trim(), which also takes off the byte order mark.
fn js_trim(text: &str) -> &str {
    text.trim_matches(|c: char| c.is_whitespace() || c == '\u{FEFF}')
}

/// One or two letters or numbers, upper case; None when the text can't be a monogram.
pub fn monogram_text(value: &str) -> Option<String> {
    let normalized: String = value.nfkc().collect();
    let text = js_trim(&normalized).to_uppercase();
    if !MONOGRAM.is_match(&text) || text.graphemes(true).count() > 2 {
        return None;
    }
    Some(text)
}

/// A saved or sent icon, checked; anything malformed is None, so it falls back to the monogram.
pub fn parse_project_icon(value: &Value) -> Option<ProjectIcon> {
    let icon = value.as_object()?;
    match icon.get("kind")?.as_str()? {
        "lucide" => {
            let name = icon.get("name")?.as_str()?;
            let color = color(icon.get("color")?)?;
            (name.len() <= 64 && LUCIDE_NAME.is_match(name)).then(|| ProjectIcon::Lucide {
                name: name.to_string(),
                color,
            })
        }
        "emoji" => {
            let emoji = first_emoji(icon.get("emoji")?.as_str()?)?;
            Some(ProjectIcon::Emoji { emoji })
        }
        "monogram" => {
            let text = monogram_text(icon.get("text")?.as_str()?)?;
            let color = color(icon.get("color")?)?;
            Some(ProjectIcon::Monogram { text, color })
        }
        _ => None,
    }
}

/// ProjectSidebarChanges.icon: left out keeps the icon; anything else is checked by parse_project_icon.
pub(crate) fn deserialize_change<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Option<ProjectIcon>>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(Some(parse_project_icon(&value)))
}

/// The first emoji in the text, or None.
pub fn first_emoji(value: &str) -> Option<String> {
    let trimmed = js_trim(value);
    let segment = trimmed.graphemes(true).next()?;
    let is_flag = FLAG.is_match(segment);
    let is_keycap = KEYCAP.is_match(segment);
    if !PICTOGRAPHIC.is_match(segment) && !is_flag && !is_keycap {
        return None;
    }
    Some(segment.to_string())
}

/// A project's monogram and color, from its name until an icon is picked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectIdentity {
    pub monogram: String,
    pub color: ProjectIconColor,
}

/// The monogram and color a project gets from its name until an icon is picked.
pub fn derive_project_identity(project_name: &str) -> ProjectIdentity {
    let normalized: String = project_name.nfkc().collect();
    let name = js_trim(&normalized);
    let words: Vec<&str> = WORD.find_iter(name).map(|word| word.as_str()).collect();
    let mut monogram = "PR".to_string();
    if let Some(first_word) = words.first() {
        let glyphs: Vec<char> = first_word.chars().collect();
        let first = glyphs[0];
        let second = glyphs[1..]
            .iter()
            .copied()
            .find(|glyph| glyph.is_numeric())
            .or_else(|| {
                if words.len() > 1 {
                    words.last().and_then(|word| word.chars().next())
                } else {
                    glyphs.last().copied()
                }
            })
            .unwrap_or(first);
        monogram = format!("{first}{second}")
            .to_uppercase()
            .chars()
            .take(2)
            .collect();
    }
    let lower = name.to_lowercase();
    let seed = if lower.is_empty() { "project" } else { &lower };
    let mut index = 0usize;
    for glyph in seed.chars() {
        index = (index * 31 + glyph as usize) % PROJECT_ICON_COLORS.len();
    }
    ProjectIdentity {
        monogram,
        color: PROJECT_ICON_COLORS[index],
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    // From src/lib/sidebar.test.ts.
    #[test]
    fn makes_a_monogram_and_a_color_from_the_name_like_t3_code() {
        assert_eq!(derive_project_identity("Wedding Jan").monogram, "WJ");
        assert_eq!(derive_project_identity("Episode 12").monogram, "E1");
        assert_eq!(derive_project_identity("").monogram, "PR");
        assert_eq!(
            derive_project_identity("Wedding Jan").color,
            derive_project_identity(" Wedding Jan ").color
        );
    }

    #[test]
    fn only_accepts_well_formed_icons() {
        let lucide = json!({ "kind": "lucide", "name": "rocket", "color": "violet" });
        assert_eq!(
            parse_project_icon(&lucide),
            Some(ProjectIcon::Lucide {
                name: "rocket".into(),
                color: ProjectIconColor::Violet
            })
        );
        assert_eq!(
            parse_project_icon(&json!({ "kind": "lucide", "name": "Rocket!", "color": "violet" })),
            None
        );
        assert_eq!(
            parse_project_icon(&json!({ "kind": "monogram", "text": "ab", "color": "red" })),
            Some(ProjectIcon::Monogram {
                text: "AB".into(),
                color: ProjectIconColor::Red
            })
        );
        assert_eq!(
            parse_project_icon(&json!({ "kind": "monogram", "text": "abc", "color": "red" })),
            None
        );
        assert_eq!(
            parse_project_icon(&json!({ "kind": "emoji", "emoji": "🎬 film" })),
            Some(ProjectIcon::Emoji {
                emoji: "🎬".into()
            })
        );
        assert_eq!(
            parse_project_icon(&json!({ "kind": "emoji", "emoji": "film" })),
            None
        );
        assert_eq!(parse_project_icon(&json!("rocket")), None);
        assert_eq!(monogram_text(" x ").as_deref(), Some("X"));
        assert_eq!(first_emoji("🇳🇱").as_deref(), Some("🇳🇱"));
    }

    // What the TypeScript gives for the same names, so the colors in the sidebar don't change with the port.
    #[test]
    fn matches_the_typescript_identities() {
        let cases = [
            ("Wedding Jan", "WJ", ProjectIconColor::Fuchsia),
            ("Episode 12", "E1", ProjectIconColor::Gray),
            ("", "PR", ProjectIconColor::Fuchsia),
            ("café ünïcode!", "CÜ", ProjectIconColor::Indigo),
            ("ﬁlm", "FM", ProjectIconColor::Sky),
            ("straße", "SE", ProjectIconColor::Yellow),
            ("ß", "SS", ProjectIconColor::Emerald),
            ("🎬 Promo", "PO", ProjectIconColor::Amber),
        ];
        for (name, monogram, color) in cases {
            assert_eq!(
                derive_project_identity(name),
                ProjectIdentity {
                    monogram: monogram.into(),
                    color
                },
                "{name}"
            );
        }
        assert_eq!(monogram_text("é"), Some("É".into()));
        assert_eq!(monogram_text("1️⃣"), Some("1️⃣".into()));
        assert_eq!(first_emoji("1️⃣ one").as_deref(), Some("1️⃣"));
        assert_eq!(first_emoji("👩‍👩‍👧 family").as_deref(), Some("👩‍👩‍👧"));
    }
}
