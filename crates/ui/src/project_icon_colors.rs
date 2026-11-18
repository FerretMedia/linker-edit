//! T3 Code's project icon colors (apps/web/src/projectIconColors.ts; MIT, see NOTICE), as
//! src/lib/projectIconColors.ts has them: the colors, the popular Lucide icons and the emoji.
//!
//! Tailwind's classes are resolved to sRGB here, the way the browser draws them: a glyph is
//! `text-<color>-600` in light and `-400` in dark, a swatch `bg-<color>-500` (Tailwind v4's
//! palette, from tailwindcss/theme.css).

use gpui::{Hsla, rgb};
use linker_edit_protocol::ProjectIconColor;

use crate::theme::Theme;

pub struct ColorOption {
    pub value: ProjectIconColor,
    pub label: &'static str,
    /// -600, -400 and -500.
    light: u32,
    dark: u32,
    swatch: u32,
}

macro_rules! options {
    ($($value:ident $label:literal $light:literal $dark:literal $swatch:literal,)*) => {
        pub const PROJECT_ICON_COLOR_OPTIONS: &[ColorOption] = &[
            $(ColorOption { value: ProjectIconColor::$value, label: $label, light: $light, dark: $dark, swatch: $swatch },)*
        ];
    };
}

options! {
    Gray "Gray" 0x4a5565 0x99a1af 0x6a7282,
    Red "Red" 0xe7000b 0xff6467 0xfb2c36,
    Orange "Orange" 0xf54900 0xff8904 0xff6900,
    Amber "Amber" 0xe17100 0xffb900 0xfe9a00,
    Yellow "Yellow" 0xd08700 0xfdc700 0xf0b100,
    Lime "Lime" 0x5ea500 0x9ae600 0x7ccf00,
    Green "Green" 0x00a63e 0x05df72 0x00c950,
    Emerald "Emerald" 0x009966 0x00d492 0x00bc7d,
    Teal "Teal" 0x009689 0x00d5be 0x00bba7,
    Cyan "Cyan" 0x0092b8 0x00d3f2 0x00b8db,
    Sky "Sky" 0x0084d1 0x00bcff 0x00a6f4,
    Blue "Blue" 0x155dfc 0x51a2ff 0x2b7fff,
    Indigo "Indigo" 0x4f39f6 0x7c86ff 0x615fff,
    Violet "Violet" 0x7f22fe 0xa684ff 0x8e51ff,
    Purple "Purple" 0x9810fa 0xc27aff 0xad46ff,
    Fuchsia "Fuchsia" 0xc800de 0xed6aff 0xe12afb,
    Pink "Pink" 0xe60076 0xfb64b6 0xf6339a,
    Rose "Rose" 0xec003f 0xff637e 0xff2056,
}

fn option(color: ProjectIconColor) -> &'static ColorOption {
    PROJECT_ICON_COLOR_OPTIONS
        .iter()
        .find(|option| option.value == color)
        .unwrap_or(&PROJECT_ICON_COLOR_OPTIONS[0])
}

/// colorClass: the glyph's color in this theme.
pub fn color(color: ProjectIconColor, theme: &Theme) -> Hsla {
    let option = option(color);
    rgb(if theme.dark {
        option.dark
    } else {
        option.light
    })
    .into()
}

/// swatchClassName.
pub fn swatch(color: ProjectIconColor) -> Hsla {
    rgb(option(color).swatch).into()
}

pub const POPULAR_PROJECT_ICONS: [&str; 24] = [
    "folder-code",
    "clapperboard",
    "film",
    "video",
    "camera",
    "mic",
    "music",
    "image",
    "palette",
    "sparkles",
    "rocket",
    "heart",
    "star",
    "globe-2",
    "book-open",
    "gamepad-2",
    "shopping-bag",
    "briefcase",
    "graduation-cap",
    "trophy",
    "plane",
    "car",
    "house",
    "layers-3",
];

pub const PROJECT_EMOJIS: [(&str, &str); 30] = [
    ("🎬", "Movie"),
    ("🎥", "Camera"),
    ("📹", "Video camera"),
    ("🎙️", "Microphone"),
    ("🎵", "Music"),
    ("🎞️", "Film frames"),
    ("📺", "TV"),
    ("🖼️", "Picture"),
    ("🚀", "Rocket"),
    ("✨", "Sparkles"),
    ("⚡", "Lightning"),
    ("🔥", "Fire"),
    ("💡", "Idea"),
    ("💍", "Ring"),
    ("🎉", "Party"),
    ("🏆", "Trophy"),
    ("🌍", "World"),
    ("✈️", "Plane"),
    ("🏠", "House"),
    ("🛍️", "Shopping"),
    ("🎮", "Game"),
    ("📚", "Books"),
    ("🧠", "Brain"),
    ("📊", "Chart"),
    ("💼", "Briefcase"),
    ("🌱", "Seedling"),
    ("🦄", "Unicorn"),
    ("🐙", "Octopus"),
    ("❤️", "Heart"),
    ("⭐", "Star"),
];

#[cfg(test)]
mod tests {
    use super::*;
    use linker_edit_protocol::PROJECT_ICON_COLORS;

    #[test]
    fn has_every_color_of_the_protocol_in_its_order() {
        let values: Vec<_> = PROJECT_ICON_COLOR_OPTIONS.iter().map(|o| o.value).collect();
        assert_eq!(values, PROJECT_ICON_COLORS);
    }
}
