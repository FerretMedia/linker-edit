//! The look: apps/desktop/src/app.css's tokens (T3 Code's palette and geometry), light and dark.
//!
//! Colors are the CSS values resolved to sRGB the way a browser does (oklch, color-mix), so a
//! token here and in app.css is the same pixel. Read them with `Theme::of(cx)`; a view never
//! hard-codes a color. Tailwind's `text-muted-foreground/70` is `theme.muted_foreground.opacity(0.7)`.
//! When app.css changes a token, change it here too.
//!
//! The theme follows the OS (like the browser's "System" choice). `LINKER_EDIT_THEME=light|dark`
//! forces one, for screenshots and tests; the Settings page will set `Theme::choose` later.

use gpui::{App, Global, Hsla, Pixels, Styled, Window, WindowAppearance, px, rgba};
use gpui_kit::component::theme::{Theme as ComponentTheme, ThemeMode};

/// Light, dark, or whatever the OS is set to; like src/lib/theme.ts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Choice {
    #[default]
    System,
    Light,
    Dark,
}

impl Choice {
    fn from_env() -> Self {
        match std::env::var("LINKER_EDIT_THEME").as_deref() {
            Ok("light") => Choice::Light,
            Ok("dark") => Choice::Dark,
            _ => Choice::System,
        }
    }

    fn is_dark(self, appearance: WindowAppearance) -> bool {
        match self {
            Choice::Light => false,
            Choice::Dark => true,
            Choice::System => matches!(
                appearance,
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        }
    }
}

/// Tailwind's text sizes as app.css uses them; each sets the size and its line height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TextSize {
    /// text-3xs, 10px
    X3s,
    /// text-2xs, 11px
    X2s,
    /// text-xs, 12px
    Xs,
    /// text-sm, 14px: most of the UI
    Sm,
    /// text-[0.9375rem], 15px: the composer
    Composer,
    /// text-base, 16px
    Base,
    /// text-lg, 18px
    Lg,
    /// text-xl, 20px
    Xl,
    /// text-2xl, 24px
    X2l,
}

impl TextSize {
    pub fn size(self) -> Pixels {
        px(match self {
            TextSize::X3s => 10.,
            TextSize::X2s => 11.,
            TextSize::Xs => 12.,
            TextSize::Sm => 14.,
            TextSize::Composer => 15.,
            TextSize::Base => 16.,
            TextSize::Lg => 18.,
            TextSize::Xl => 20.,
            TextSize::X2l => 24.,
        })
    }

    pub fn line_height(self) -> Pixels {
        px(match self {
            TextSize::X3s => 14.,
            TextSize::X2s => 16.,
            TextSize::Xs => 16.,
            TextSize::Sm => 20.,
            // leading-relaxed
            TextSize::Composer => 15. * 1.625,
            TextSize::Base => 24.,
            TextSize::Lg => 28.,
            TextSize::Xl => 28.,
            TextSize::X2l => 32.,
        })
    }
}

/// `.text_token(TextSize::Sm)` instead of gpui's `.text_sm()`, which sets only the size.
pub trait TextStyled: Styled + Sized {
    fn text_token(self, size: TextSize) -> Self {
        self.text_size(size.size()).line_height(size.line_height())
    }
}

impl<T: Styled> TextStyled for T {}

/// app.css's radii (`--radius` is 0.5rem). Use these, not gpui's `rounded_sm()` (2px, not 4px).
#[derive(Clone, Copy, Debug)]
pub struct Radii {
    pub sm: Pixels,
    pub md: Pixels,
    pub lg: Pixels,
    pub xl: Pixels,
    pub xl2: Pixels,
    /// --control-radius: sidebar rows and buttons
    pub control: Pixels,
}

/// One theme's tokens; field names follow app.css's custom properties.
#[derive(Clone, Debug)]
pub struct Theme {
    pub dark: bool,
    pub background: Hsla,
    pub foreground: Hsla,
    pub card: Hsla,
    pub primary: Hsla,
    pub primary_foreground: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub accent: Hsla,
    /// The user's chat bubble.
    pub message: Hsla,
    pub border: Hsla,
    pub input: Hsla,
    pub destructive: Hsla,
    pub success: Hsla,
    pub success_foreground: Hsla,
    pub warning: Hsla,
    pub warning_foreground: Hsla,
    pub info: Hsla,
    pub info_foreground: Hsla,
    pub popover: Hsla,
    pub ring: Hsla,
    pub sidebar: Hsla,
    pub sidebar_foreground: Hsla,
    pub sidebar_muted_foreground: Hsla,
    pub sidebar_row_active: Hsla,
    pub sidebar_row_hover: Hsla,
    pub sidebar_row_selected: Hsla,
    pub sidebar_border: Hsla,
    /// --sidebar-icon-color
    pub sidebar_icon: Hsla,
    pub composer_outline: Hsla,
    pub scrollbar_thumb: Hsla,
    pub radius: Radii,
    /// The UI font: the system's, as app.css's font stack lands on (Segoe UI on Windows).
    pub font_family: &'static str,
    pub mono_font_family: &'static str,
}

fn c(hex: u32) -> Hsla {
    rgba(hex).into()
}

const RADII: Radii = Radii {
    sm: px(4.),
    md: px(6.),
    lg: px(8.),
    xl: px(12.),
    xl2: px(16.),
    control: px(8.),
};

const MONO: &str = if cfg!(target_os = "windows") {
    "Consolas"
} else if cfg!(target_os = "macos") {
    "Menlo"
} else {
    "DejaVu Sans Mono"
};

impl Theme {
    /// app.css's `:root`.
    pub fn light() -> Self {
        Theme {
            dark: false,
            background: c(0xfcfcfcff),
            foreground: c(0x18181bff),
            card: c(0xffffffff),
            primary: c(0x1b4ed8ff),
            primary_foreground: c(0xffffffff),
            muted: c(0xfafafaff),
            muted_foreground: c(0x71717bff),
            accent: c(0xf4f4f5ff),
            message: c(0xebebedff),
            border: c(0xe4e4e7ff),
            input: c(0xd4d4d8ff),
            destructive: c(0xfb2c36ff),
            success: c(0x00bc7dff),
            success_foreground: c(0x007a55ff),
            warning: c(0xfe9a00ff),
            warning_foreground: c(0xbb4d00ff),
            info: c(0x2b7fffff),
            info_foreground: c(0x1447e6ff),
            popover: c(0xffffffff),
            ring: c(0x1b4ed8ff),
            sidebar: c(0xfafafaff),
            sidebar_foreground: c(0x18181bff),
            sidebar_muted_foreground: c(0x71717bff),
            sidebar_row_active: c(0xffffffff),
            sidebar_row_hover: c(0xfcfcfcff),
            sidebar_row_selected: c(0xffffffff),
            sidebar_border: c(0xe4e4e7ff),
            sidebar_icon: c(0x84848dff),
            composer_outline: c(0x00000014),
            scrollbar_thumb: c(0xd9d9d9ff),
            radius: RADII,
            font_family: ".SystemUIFont",
            mono_font_family: MONO,
        }
    }

    /// app.css's `.dark`.
    pub fn dark() -> Self {
        Theme {
            dark: true,
            background: c(0x000000ff),
            foreground: c(0xf5f5f5ff),
            card: c(0x111111ff),
            primary: c(0x346bf1ff),
            primary_foreground: c(0xffffffff),
            muted: c(0xffffff08),
            muted_foreground: c(0xa1a1a1ff),
            accent: c(0xffffff0a),
            message: c(0x1e1e1eff),
            border: c(0xffffff14),
            input: c(0xffffff1c),
            destructive: c(0xfb414aff),
            success: c(0x00bc7dff),
            success_foreground: c(0x00d492ff),
            warning: c(0xfe9a00ff),
            warning_foreground: c(0xffb900ff),
            info: c(0x2b7fffff),
            info_foreground: c(0x51a2ffff),
            popover: c(0x111111ff),
            ring: c(0x346bf1ff),
            sidebar: c(0x111111ff),
            sidebar_foreground: c(0xf5f5f5ff),
            sidebar_muted_foreground: c(0xa1a1a1ff),
            sidebar_row_active: c(0xffffff0a),
            sidebar_row_hover: c(0xffffff0a),
            sidebar_row_selected: c(0xffffff08),
            sidebar_border: c(0xffffff14),
            sidebar_icon: c(0x898989ff),
            composer_outline: c(0xffffff0d),
            scrollbar_thumb: c(0xffffff14),
            radius: RADII,
            font_family: ".SystemUIFont",
            mono_font_family: MONO,
        }
    }

    pub fn of(cx: &App) -> &Theme {
        &cx.global::<Current>().theme
    }

    /// Installs the theme for this window's appearance and keeps it in step with the OS.
    pub fn init(window: &mut Window, cx: &mut App) {
        let choice = Choice::from_env();
        set(choice, window.appearance(), cx);
        window
            .observe_window_appearance(|window, cx| {
                let choice = cx.global::<Current>().choice;
                set(choice, window.appearance(), cx);
            })
            .detach();
    }

    /// Picks light, dark or the OS's; what the Settings page will call.
    pub fn choose(choice: Choice, window: &mut Window, cx: &mut App) {
        set(choice, window.appearance(), cx);
    }
}

struct Current {
    choice: Choice,
    theme: Theme,
}

impl Global for Current {}

fn set(choice: Choice, appearance: WindowAppearance, cx: &mut App) {
    let dark = choice.is_dark(appearance);
    let theme = if dark { Theme::dark() } else { Theme::light() };
    // gpui-component's controls (inputs, menus, dialogs, lists) read its own theme: point it
    // at our tokens so they look like the rest. Switch the mode first; it reloads its colors.
    ComponentTheme::change(
        if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        },
        None,
        cx,
    );
    ComponentTheme::update(cx, |t| {
        t.font_family = theme.font_family.into();
        t.mono_font_family = theme.mono_font_family.into();
        t.font_size = TextSize::Sm.size();
        t.radius = theme.radius.md;
        t.radius_lg = theme.radius.lg;
        t.shadow = !dark;
        t.colors.background = theme.background;
        t.colors.foreground = theme.foreground;
        t.colors.border = theme.border;
        t.colors.input = theme.input;
        t.colors.ring = theme.ring;
        t.colors.caret = theme.foreground;
        t.colors.selection = theme.primary.opacity(0.25);
        t.colors.primary = theme.primary;
        t.colors.primary_hover = theme.primary.opacity(0.9);
        t.colors.primary_active = theme.primary.opacity(0.8);
        t.colors.primary_foreground = theme.primary_foreground;
        t.colors.secondary = theme.card;
        t.colors.secondary_hover = theme.accent;
        t.colors.secondary_active = theme.accent;
        t.colors.secondary_foreground = theme.foreground;
        t.colors.muted = theme.muted;
        t.colors.muted_foreground = theme.muted_foreground;
        t.colors.accent = theme.accent;
        t.colors.accent_foreground = theme.foreground;
        t.colors.popover = theme.popover;
        t.colors.popover_foreground = theme.foreground;
        t.colors.list = theme.background;
        t.colors.list_hover = theme.accent;
        t.colors.list_active = theme.accent;
        t.colors.list_active_border = theme.border;
        t.colors.sidebar = theme.sidebar;
        t.colors.sidebar_foreground = theme.sidebar_foreground;
        t.colors.sidebar_border = theme.sidebar_border;
        t.colors.sidebar_accent = theme.sidebar_row_hover;
        t.colors.sidebar_accent_foreground = theme.sidebar_foreground;
        t.colors.sidebar_primary = theme.primary;
        t.colors.sidebar_primary_foreground = theme.primary_foreground;
        t.colors.danger = theme.destructive;
        t.colors.danger_foreground = theme.primary_foreground;
        t.colors.success = theme.success;
        t.colors.warning = theme.warning;
        t.colors.info = theme.info;
        t.colors.link = theme.primary;
        t.colors.scrollbar_thumb = theme.scrollbar_thumb;
        t.colors.title_bar = theme.background;
        t.colors.title_bar_border = theme.border;
        t.colors.window_border = theme.border;
    });
    cx.set_global(Current { choice, theme });
    cx.refresh_windows();
}

/// The root's text defaults, like app.css's `body`.
pub fn root_text<E: Styled>(element: E, theme: &Theme) -> E {
    element
        .font_family(theme.font_family)
        .text_color(theme.foreground)
        .text_token(TextSize::Sm)
}
