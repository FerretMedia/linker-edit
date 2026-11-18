//! src/lib/collectionIcons.ts and CollectionIcon.svelte: a workspace's or series' icon (a
//! Lucide icon, an emoji or its letters) on its color.

use gpui::{
    App, FontWeight, Hsla, IntoElement, ParentElement, RenderOnce, Styled, Window, div, px, rgb,
};
use gpui_kit::component::Icon;
use linker_edit_protocol::{CollectionIcon, IconColor};

use crate::icons::IconName;
use crate::theme::{TextSize, TextStyled, Theme};

/// The Lucide icons a workspace or series can have, by their Lucide name.
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

/// ICON_SWATCH_CLASS: Tailwind's -500 of each color.
pub fn swatch(color: IconColor) -> Hsla {
    rgb(match color {
        IconColor::Gray => 0x71717b,
        IconColor::Red => 0xfb2c36,
        IconColor::Orange => 0xff6900,
        IconColor::Amber => 0xfe9a00,
        IconColor::Green => 0x00bc7d,
        IconColor::Teal => 0x00bba7,
        IconColor::Blue => 0x2b7fff,
        IconColor::Violet => 0x8e51ff,
        IconColor::Pink => 0xf6339a,
    })
    .into()
}

/// ICON_COLOR_CLASS: the tile (`bg-<color>-500/15`) and the glyph's color, readable in light and
/// dark.
pub fn tile_colors(color: IconColor, dark: bool) -> (Hsla, Hsla) {
    let glyph = match (color, dark) {
        (IconColor::Gray, false) => 0x52525c,
        (IconColor::Gray, true) => 0xd4d4d8,
        (IconColor::Red, false) => 0xe7000b,
        (IconColor::Red, true) => 0xff6467,
        (IconColor::Orange, false) => 0xf54900,
        (IconColor::Orange, true) => 0xff8904,
        (IconColor::Amber, false) => 0xbb4d00,
        (IconColor::Amber, true) => 0xffb900,
        (IconColor::Green, false) => 0x007a55,
        (IconColor::Green, true) => 0x00d492,
        (IconColor::Teal, false) => 0x00786f,
        (IconColor::Teal, true) => 0x00d5be,
        (IconColor::Blue, false) => 0x155dfc,
        (IconColor::Blue, true) => 0x51a2ff,
        (IconColor::Violet, false) => 0x7f22fe,
        (IconColor::Violet, true) => 0xa684ff,
        (IconColor::Pink, false) => 0xe60076,
        (IconColor::Pink, true) => 0xfb64b6,
    };
    (swatch(color).opacity(0.15), rgb(glyph).into())
}

/// "Ferret Media" → "FM", "podcast" → "P": a monogram's letters when none were typed.
pub fn monogram(name: &str) -> String {
    let words: Vec<&str> = name.split_whitespace().collect();
    let letters: String = if words.len() > 1 {
        words
            .iter()
            .take(2)
            .filter_map(|word| word.chars().next())
            .collect()
    } else {
        words
            .first()
            .and_then(|word| word.chars().next())
            .unwrap_or('?')
            .to_string()
    };
    let letters = letters.to_uppercase();
    if letters.is_empty() {
        "?".into()
    } else {
        letters
    }
}

pub fn icon_text(icon: &CollectionIcon, name: &str) -> String {
    match icon {
        CollectionIcon::Monogram { text, .. } if !text.trim().is_empty() => text.trim().into(),
        CollectionIcon::Monogram { .. } => monogram(name),
        _ => String::new(),
    }
}

pub fn icon_color(icon: &CollectionIcon) -> IconColor {
    match icon {
        CollectionIcon::Lucide { color, .. }
        | CollectionIcon::Emoji { color, .. }
        | CollectionIcon::Monogram { color, .. } => *color,
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IconSize {
    Sm,
    #[default]
    Md,
    Lg,
}

/// CollectionIcon.svelte.
#[derive(IntoElement)]
pub struct CollectionIconView {
    icon: CollectionIcon,
    name: String,
    size: IconSize,
}

impl CollectionIconView {
    pub fn new(icon: &CollectionIcon, name: &str) -> Self {
        CollectionIconView {
            icon: icon.clone(),
            name: name.into(),
            size: IconSize::Md,
        }
    }

    pub fn size(mut self, size: IconSize) -> Self {
        self.size = size;
        self
    }
}

impl RenderOnce for CollectionIconView {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let (bg, fg) = tile_colors(icon_color(&self.icon), theme.dark);
        let (box_size, radius, text, glyph) = match self.size {
            IconSize::Sm => (px(20.), theme.radius.sm, TextSize::X2s, px(12.)),
            IconSize::Md => (px(24.), theme.radius.md, TextSize::Xs, px(14.)),
            IconSize::Lg => (px(36.), theme.radius.lg, TextSize::Sm, px(20.)),
        };
        let tile = div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(box_size)
            .rounded(radius)
            .bg(bg)
            .text_color(fg)
            .text_token(text)
            .font_weight(FontWeight::SEMIBOLD);
        match &self.icon {
            CollectionIcon::Lucide { name, .. } => {
                tile.child(Icon::new(lucide_icon(name)).size(glyph))
            }
            CollectionIcon::Emoji { emoji, .. } => {
                tile.child(div().line_height(text.size()).child(emoji.clone()))
            }
            CollectionIcon::Monogram { .. } => tile.child(icon_text(&self.icon, &self.name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_a_monograms_letters_from_the_name() {
        assert_eq!(monogram("Ferret Media"), "FM");
        assert_eq!(monogram("podcast"), "P");
        assert_eq!(monogram("  "), "?");
        let typed = CollectionIcon::Monogram {
            text: " ab ".into(),
            color: IconColor::Blue,
        };
        assert_eq!(icon_text(&typed, "Ferret Media"), "ab");
    }
}
