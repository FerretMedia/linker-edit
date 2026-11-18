//! Icons: the Lucide icons the Svelte UI imports, and the logos, embedded in the binary.
//!
//! Draw one with gpui-component's `Icon`, which tints it with the text color:
//! `Icon::new(IconName::History).size(px(14.)).text_color(theme.muted_foreground)`.
//! Buttons from gpui-component take them too (`Button::new("x").icon(IconName::X)`).
//!
//! To add a Lucide icon: copy its SVG from @lucide/svelte's version (apps/desktop/package.json)
//! into `assets/icons/<kebab-name>.svg` and add a line below. The SVGs here were written from
//! @lucide/svelte 1.67.0's own icon data, so they match the browser UI stroke for stroke.

use std::borrow::Cow;

use gpui::SharedString;
use gpui_kit::component::IconNamed;

macro_rules! icons {
    ($($name:ident => $file:literal,)*) => {
        /// The Lucide icons the UI uses, named as `@lucide/svelte` exports them.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum IconName {
            $($name,)*
        }

        impl IconName {
            pub const ALL: &'static [IconName] = &[$(IconName::$name,)*];

            /// The asset path, under our own prefix so gpui-component's `icons/…` stay theirs.
            pub fn path(self) -> &'static str {
                match self {
                    $(IconName::$name => concat!("le/icons/", $file, ".svg"),)*
                }
            }
        }

        pub(crate) fn icon_svg(path: &str) -> Option<&'static [u8]> {
            match path {
                $(concat!("le/icons/", $file, ".svg") => Some(include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/icons/", $file, ".svg"))),)*
                _ => None,
            }
        }
    };
}

icons! {
    AlarmClock => "alarm-clock",
    AlarmClockOff => "alarm-clock-off",
    Archive => "archive",
    ArchiveRestore => "archive-restore",
    ArrowLeft => "arrow-left",
    ArrowUp => "arrow-up",
    AudioLines => "audio-lines",
    AudioWaveform => "audio-waveform",
    BookOpen => "book-open",
    Briefcase => "briefcase",
    Building2 => "building-2",
    Camera => "camera",
    Check => "check",
    ChevronDown => "chevron-down",
    ChevronRight => "chevron-right",
    ChevronUp => "chevron-up",
    ChevronsUpDown => "chevrons-up-down",
    CircleAlert => "circle-alert",
    CircleCheck => "circle-check",
    CircleDashed => "circle-dashed",
    Clapperboard => "clapperboard",
    Clock => "clock",
    Cloud => "cloud",
    Coffee => "coffee",
    Copy => "copy",
    CornerLeftUp => "corner-left-up",
    Dumbbell => "dumbbell",
    Ellipsis => "ellipsis",
    ExternalLink => "external-link",
    FilePen => "file-pen",
    FileText => "file-text",
    Film => "film",
    Folder => "folder",
    FolderCheck => "folder-check",
    FolderCode => "folder-code",
    FolderInput => "folder-input",
    FolderOpen => "folder-open",
    FolderPlus => "folder-plus",
    FolderSearch => "folder-search",
    Gamepad2 => "gamepad-2",
    Globe => "globe",
    GraduationCap => "graduation-cap",
    HardDrive => "hard-drive",
    Hash => "hash",
    Headphones => "headphones",
    Heart => "heart",
    History => "history",
    House => "house",
    Image => "image",
    ImageOff => "image-off",
    Info => "info",
    Layers => "layers",
    Library => "library",
    ListTodo => "list-todo",
    LoaderCircle => "loader-circle",
    LogIn => "log-in",
    LogOut => "log-out",
    MailOpen => "mail-open",
    Maximize => "maximize",
    Megaphone => "megaphone",
    MessageSquarePlus => "message-square-plus",
    Mic => "mic",
    Minimize => "minimize",
    Monitor => "monitor",
    MonitorPlay => "monitor-play",
    MonitorUp => "monitor-up",
    Music => "music",
    Newspaper => "newspaper",
    Palette => "palette",
    Pause => "pause",
    // Play and Pause with `fill-current`, as the player's buttons draw them.
    PauseFilled => "pause-filled",
    Pencil => "pencil",
    Pin => "pin",
    PinOff => "pin-off",
    Plane => "plane",
    Play => "play",
    PlayFilled => "play-filled",
    Plus => "plus",
    Podcast => "podcast",
    Radio => "radio",
    RefreshCw => "refresh-cw",
    Rocket => "rocket",
    RotateCcw => "rotate-ccw",
    Scissors => "scissors",
    Search => "search",
    Server => "server",
    Settings => "settings",
    Settings2 => "settings-2",
    ShieldAlert => "shield-alert",
    ShieldCheck => "shield-check",
    ShieldOff => "shield-off",
    ShieldQuestion => "shield-question",
    SkipBack => "skip-back",
    SkipForward => "skip-forward",
    Sparkles => "sparkles",
    Square => "square",
    SquarePen => "square-pen",
    SquareTerminal => "square-terminal",
    Star => "star",
    Store => "store",
    Trash => "trash",
    Trash2 => "trash-2",
    Tv => "tv",
    Undo2 => "undo-2",
    UserRound => "user-round",
    Users => "users",
    Utensils => "utensils",
    Video => "video",
    Volume2 => "volume-2",
    VolumeX => "volume-x",
    WifiOff => "wifi-off",
    Wrench => "wrench",
    X => "x",
}

impl IconNamed for IconName {
    fn path(self) -> SharedString {
        IconName::path(self).into()
    }
}

/// Logos, drawn as single-color masks like the icons. Linker's mark is three of them
/// stacked (see `components::LinkerLogo`), since GPUI tints an SVG with one color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Logo {
    /// The two cards of Linker's mark; drawn in the text color.
    LinkerMark,
    /// The blue glow behind the cards, offset to the bottom right.
    LinkerMarkBlue,
    /// The pink glow behind the cards, offset to the top left.
    LinkerMarkPink,
    /// Claude's mark, drawn in #d97757 (T3 Code's ClaudeIcon).
    Claude,
    /// OpenAI's mark, drawn in the text color.
    OpenAi,
}

impl IconNamed for Logo {
    fn path(self) -> SharedString {
        match self {
            Logo::LinkerMark => "le/logos/linker-mark.svg",
            Logo::LinkerMarkBlue => "le/logos/linker-mark-blue.svg",
            Logo::LinkerMarkPink => "le/logos/linker-mark-pink.svg",
            Logo::Claude => "le/logos/claude.svg",
            Logo::OpenAi => "le/logos/openai.svg",
        }
        .into()
    }
}

fn logo_svg(path: &str) -> Option<&'static [u8]> {
    macro_rules! logo {
        ($file:literal) => {
            include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/logos/", $file))
        };
    }
    Some(match path {
        "le/logos/linker-mark.svg" => logo!("linker-mark.svg"),
        "le/logos/linker-mark-blue.svg" => logo!("linker-mark-blue.svg"),
        "le/logos/linker-mark-pink.svg" => logo!("linker-mark-pink.svg"),
        "le/logos/claude.svg" => logo!("claude.svg"),
        "le/logos/openai.svg" => logo!("openai.svg"),
        _ => return None,
    })
}

/// Our SVGs first, then gpui-kit's default icons, which its own components draw.
pub struct Assets;

impl gpui::AssetSource for Assets {
    fn load(&self, path: &str) -> gpui::Result<Option<Cow<'static, [u8]>>> {
        if let Some(bytes) = icon_svg(path).or_else(|| logo_svg(path)) {
            return Ok(Some(Cow::Borrowed(bytes)));
        }
        gpui_kit::assets::Assets.load(path)
    }

    fn list(&self, path: &str) -> gpui::Result<Vec<SharedString>> {
        let mut paths: Vec<SharedString> = IconName::ALL
            .iter()
            .map(|icon| icon.path())
            .filter(|p| p.starts_with(path))
            .map(SharedString::from)
            .collect();
        paths.extend(gpui_kit::assets::Assets.list(path)?);
        Ok(paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AssetSource as _;

    #[test]
    fn every_icon_and_logo_is_embedded() {
        for icon in IconName::ALL {
            assert!(
                Assets.load(icon.path()).unwrap().is_some(),
                "{icon:?} has no SVG"
            );
        }
        for logo in [
            Logo::LinkerMark,
            Logo::LinkerMarkBlue,
            Logo::LinkerMarkPink,
            Logo::Claude,
            Logo::OpenAi,
        ] {
            assert!(Assets.load(&logo.path()).unwrap().is_some());
        }
    }
}
