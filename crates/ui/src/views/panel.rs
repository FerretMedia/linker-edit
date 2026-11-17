//! The side panels, one at a time, opened from the chat header like T3 Code's diff panel:
//! Preview (PreviewPanel.svelte, 32rem), History (HistoryPanel.svelte, 24rem) and the
//! project file (ProjectFilePanel.svelte, 24rem). Each has a 52px header that is part of the
//! title bar and a close button. Placeholder bodies until their views are ported.

use gpui::{
    App, ClickEvent, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;

use crate::components::{OnClick, placeholder};
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

/// The open panel: `aside.flex w-[…] shrink-0 flex-col border-l bg-sidebar`.
#[derive(IntoElement)]
pub struct SidePanel {
    panel: Panel,
    on_close: OnClick,
}

impl SidePanel {
    pub fn new(
        panel: Panel,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        SidePanel {
            panel,
            on_close: Box::new(on_close),
        }
    }
}

impl RenderOnce for SidePanel {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let id = SharedString::from(format!("{}-panel-header", self.panel.id()));
        let header = title_bar(id, px(16.), false, true, window)
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_token(TextSize::Sm)
                    .font_weight(FontWeight::MEDIUM)
                    .child(self.panel.label()),
            )
            .child(
                no_drag().child(
                    div()
                        .id("close-panel")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size_7()
                        .rounded(theme.radius.md)
                        .text_color(theme.muted_foreground)
                        .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
                        .on_click(self.on_close)
                        .child(Icon::new(IconName::X).size(px(16.)))
                        .test_support(),
                ),
            );
        div()
            .id(SharedString::from(format!("{}-panel", self.panel.id())))
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(self.panel.width())
            .h_full()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .child(header)
            .child(placeholder(format!("{} goes here", self.panel.label()), cx))
            .test_support()
    }
}
