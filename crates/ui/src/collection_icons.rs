//! src/lib/collectionIcons.ts: the icons a workspace or series can have, their colors (Tailwind's
//! classes resolved to sRGB, light and dark), and their letters.

use gpui::{Hsla, rgb};
use linker_edit_protocol::{CollectionIcon, IconColor};

use crate::icons::IconName;
use crate::theme::Theme;

/// The lucide icons a workspace or series can have, by their lucide name.
pub const LUCIDE_ICONS: [(&str, IconName); 30] = [
    ("folder", IconName::Folder),
    ("briefcase", IconName::Briefcase),
    ("building-2", IconName::Building2),
    ("store", IconName::Store),
    ("users", IconName::Users),
    ("megaphone", IconName::Megaphone),
    ("mic", IconName::Mic),
    ("podcast", IconName::Podcast),
    ("radio", IconName::Radio),
    ("headphones", IconName::Headphones),
    ("music", IconName::Music),
    ("clapperboard", IconName::Clapperboard),
    ("film", IconName::Film),
    ("video", IconName::Video),
    ("camera", IconName::Camera),
    ("tv", IconName::Tv),
    ("gamepad-2", IconName::Gamepad2),
    ("newspaper", IconName::Newspaper),
    ("book-open", IconName::BookOpen),
    ("graduation-cap", IconName::GraduationCap),
    ("palette", IconName::Palette),
    ("sparkles", IconName::Sparkles),
    ("star", IconName::Star),
    ("heart", IconName::Heart),
    ("rocket", IconName::Rocket),
    ("globe", IconName::Globe),
    ("plane", IconName::Plane),
    ("coffee", IconName::Coffee),
    ("utensils", IconName::Utensils),
    ("dumbbell", IconName::Dumbbell),
];

/// A lucide name's icon; Folder for one that isn't offered.
pub fn lucide_icon(name: &str) -> IconName {
    LUCIDE_ICONS
        .iter()
        .find(|(lucide, _)| *lucide == name)
        .map_or(IconName::Folder, |(_, icon)| *icon)
}

pub const EMOJI_SUGGESTIONS: [&str; 12] = [
    "🎙️", "🎬", "📺", "🎵", "🚀", "💍", "🍳", "🎮", "📚", "🏋️", "✈️", "⭐",
];

pub const ICON_COLORS: [IconColor; 9] = [
    IconColor::Gray,
    IconColor::Red,
    IconColor::Orange,
    IconColor::Amber,
    IconColor::Green,
    IconColor::Teal,
    IconColor::Blue,
    IconColor::Violet,
    IconColor::Pink,
];

/// ICON_SWATCH_CLASS's -500, which the tile also is at 15%; and ICON_COLOR_CLASS's glyph, light and dark.
fn palette(color: IconColor) -> (u32, u32, u32) {
    match color {
        IconColor::Gray => (0x71717b, 0x52525c, 0xd4d4d8),
        IconColor::Red => (0xfb2c36, 0xe7000b, 0xff6467),
        IconColor::Orange => (0xff6900, 0xf54900, 0xff8904),
        IconColor::Amber => (0xfe9a00, 0xbb4d00, 0xffb900),
        IconColor::Green => (0x00bc7d, 0x007a55, 0x00d492),
        IconColor::Teal => (0x00bba7, 0x00786f, 0x00d5be),
        IconColor::Blue => (0x2b7fff, 0x155dfc, 0x51a2ff),
        IconColor::Violet => (0x8e51ff, 0x7f22fe, 0xa684ff),
        IconColor::Pink => (0xf6339a, 0xe60076, 0xfb64b6),
    }
}

/// The swatch for picking a color.
pub fn swatch(color: IconColor) -> Hsla {
    rgb(palette(color).0).into()
}

/// The tile behind the glyph: the swatch at 15%.
pub fn tile(color: IconColor) -> Hsla {
    swatch(color).opacity(0.15)
}

/// The glyph's color, readable in light and dark.
pub fn glyph(color: IconColor, theme: &Theme) -> Hsla {
    let (_, light, dark) = palette(color);
    rgb(if theme.dark { dark } else { light }).into()
}

pub fn color_name(color: IconColor) -> &'static str {
    match color {
        IconColor::Gray => "gray",
        IconColor::Red => "red",
        IconColor::Orange => "orange",
        IconColor::Amber => "amber",
        IconColor::Green => "green",
        IconColor::Teal => "teal",
        IconColor::Blue => "blue",
        IconColor::Violet => "violet",
        IconColor::Pink => "pink",
    }
}

pub fn icon_color(icon: &CollectionIcon) -> IconColor {
    match icon {
        CollectionIcon::Lucide { color, .. }
        | CollectionIcon::Emoji { color, .. }
        | CollectionIcon::Monogram { color, .. } => *color,
    }
}

/// "Ferret Media" → "FM", "podcast" → "P": a monogram's letters when none were typed.
pub fn monogram(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let letters: String = if words.len() > 1 {
        words[..2]
            .iter()
            .filter_map(|word| word.chars().next())
            .collect()
    } else {
        words
            .first()
            .copied()
            .unwrap_or("?")
            .chars()
            .take(1)
            .collect()
    };
    let upper = letters.to_uppercase();
    if upper.is_empty() { "?".into() } else { upper }
}

pub fn icon_text(icon: &CollectionIcon, name: &str) -> String {
    match icon {
        CollectionIcon::Monogram { text, .. } if !text.trim().is_empty() => text.trim().into(),
        CollectionIcon::Monogram { .. } => monogram(name),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn makes_letters_from_the_name() {
        assert_eq!(monogram("Ferret Media"), "FM");
        assert_eq!(monogram("podcast"), "P");
        assert_eq!(monogram("  "), "?");
        let typed = CollectionIcon::Monogram {
            text: " ab ".into(),
            color: IconColor::Blue,
        };
        assert_eq!(icon_text(&typed, "Ferret Media"), "ab");
        let empty = CollectionIcon::Monogram {
            text: "".into(),
            color: IconColor::Blue,
        };
        assert_eq!(icon_text(&empty, "Ferret Media"), "FM");
        assert_eq!(lucide_icon("nope"), IconName::Folder);
    }
}
