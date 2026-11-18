//! The side panels, one at a time, opened from the chat header like T3 Code's diff panel:
//! Preview (PreviewPanel.svelte, 32rem), History (HistoryPanel.svelte, 24rem) and the
//! project file (ProjectFilePanel.svelte, 24rem). Each has a 52px header that is part of the
//! title bar and a close button: the frame and header here, the bodies in their own views.

use gpui::{
    App, ClickEvent, Div, FontWeight, InteractiveElement, ParentElement, Pixels, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;

use crate::icons::IconName;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::window::{no_drag, title_bar};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Panel {
    Preview,
    History,
    File,
}

impl Panel {
    pub const ALL: [Panel; 3] = [Panel::Preview, Panel::History, Panel::File];

    pub fn label(self) -> &'static str {
        match self {
            Panel::Preview => "Preview",
            Panel::History => "History",
            Panel::File => "Project file",
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            Panel::Preview => IconName::MonitorPlay,
            Panel::History => IconName::History,
            Panel::File => IconName::FileText,
        }
    }

    pub fn width(self) -> Pixels {
        match self {
            Panel::Preview => px(512.),
            Panel::History | Panel::File => px(384.),
        }
    }

    /// The element id of its toggle in the chat header, and of the panel itself.
    pub fn id(self) -> &'static str {
        match self {
            Panel::Preview => "preview",
            Panel::History => "history",
            Panel::File => "file",
        }
    }
}

/// The panel's frame: `aside.flex w-[…] shrink-0 flex-col border-l bg-sidebar`, with the panel's
/// id ("history-panel").
pub fn panel_frame(panel: Panel, theme: &Theme) -> Stateful<Div> {
    div()
        .id(SharedString::from(format!("{}-panel", panel.id())))
        .flex()
        .flex_col()
        .flex_shrink_0()
        .w(panel.width())
        .h_full()
        .border_l_1()
        .border_color(theme.border)
        .bg(theme.sidebar)
}

/// The panel's 52px header, part of the title bar: its icon and `title`. The caller adds what
/// else it shows, then [`close_button`].
pub fn panel_header(
    panel: Panel,
    title: &'static str,
    window: &mut Window,
    theme: &Theme,
) -> Stateful<Div> {
    let id = SharedString::from(format!("{}-panel-header", panel.id()));
    title_bar(id, px(16.), false, true, window)
        .gap_2()
        .border_b_1()
        .border_color(theme.border)
        .child(
            Icon::new(panel.icon())
                .size(px(16.))
                .text_color(theme.muted_foreground),
        )
        .child(
            div()
                .text_token(TextSize::Sm)
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
}

/// The header's close button, at its right end.
pub fn close_button(
    tip: &'static str,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Div {
    let (accent, foreground) = (theme.accent, theme.foreground);
    no_drag().ml_auto().child(
        div()
            .id("close-panel")
            .flex()
            .items_center()
            .justify_center()
            .size_7()
            .rounded(theme.radius.md)
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .hover(move |style| style.bg(accent).text_color(foreground))
            .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
            .on_click(on_click)
            .child(Icon::new(IconName::X).size(px(16.)))
            .test_support(),
    )
}
