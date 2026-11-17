//! The chat column's body (+page.svelte's `<main>` under its header): the messages, then the
//! approvals and the composer, both 46rem wide at most and centered. The header is drawn by
//! the workspace, which owns the side panel its toggles open.
//! Placeholder content: a new thread's "What are we editing?" and a composer that can't type yet.

use gpui::{
    Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;

use crate::icons::{IconName, Logo};
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};

/// The column's `max-w-[46rem]`.
pub const CHAT_MAX_WIDTH: gpui::Pixels = px(736.);

const SUGGESTIONS: [&str; 3] = [
    "Describe the open project and its timelines",
    "What's in the media pool?",
    "Duplicate the current timeline and remove the silences",
];

pub struct Chat {
    state: Entity<AppState>,
}

impl Chat {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Chat { state }
    }
}

impl Render for Chat {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let _state = self.state.read(cx);

        let suggestions = SUGGESTIONS.iter().enumerate().map(|(index, text)| {
            div()
                .id(("suggestion", index))
                .px_3()
                .py_1p5()
                .rounded_full()
                .border_1()
                .border_color(theme.border)
                .text_token(TextSize::Sm)
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
                .child(*text)
        });

        let empty = div()
            .flex()
            .flex_col()
            .items_center()
            .gap_5()
            .py_10()
            .my_auto()
            .child(
                div()
                    .text_token(TextSize::X2l)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child("What are we editing?"),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .justify_center()
                    .gap_2()
                    .max_w(px(576.))
                    .children(suggestions),
            );

        let messages = div()
            .id("chat-messages")
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_4()
                    .size_full()
                    .max_w(CHAT_MAX_WIDTH)
                    .mx_auto()
                    .px_5()
                    .py_6()
                    .child(empty),
            );

        div()
            .id("chat")
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .child(messages)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .w_full()
                    .max_w(CHAT_MAX_WIDTH)
                    .mx_auto()
                    .px_5()
                    .pb_4()
                    .flex_shrink_0()
                    .child(composer(&theme)),
            )
            .test_support()
    }
}

/// Composer.svelte, closed and empty: the text area, then the provider, the approval mode
/// and the send button.
fn composer(theme: &Theme) -> impl IntoElement {
    let pill = |id: &'static str| {
        div()
            .id(id)
            .flex()
            .items_center()
            .gap_1p5()
            .h_7()
            .px_2()
            .rounded(theme.radius.md)
            .text_token(TextSize::Sm)
            .text_color(theme.muted_foreground)
            .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
    };
    div()
        .id("composer")
        .flex()
        .flex_col()
        .rounded(theme.radius.xl2)
        .border_1()
        .border_color(theme.composer_outline)
        .bg(theme.card)
        // shadow-composer dark:shadow-none
        .when(!theme.dark, |this| {
            this.shadow(vec![gpui::BoxShadow {
                color: gpui::hsla(0., 0., 0., 0.05),
                offset: gpui::point(px(0.), px(1.)),
                blur_radius: px(2.),
                spread_radius: px(0.),
                inset: false,
            }])
        })
        .child(
            div()
                .min_h_16()
                .px_4()
                .pt_3p5()
                .pb_1()
                .text_token(TextSize::Composer)
                .text_color(theme.muted_foreground.opacity(0.7))
                .child("Ask Claude to edit…"),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .pb_2()
                .child(
                    pill("provider")
                        .child(
                            Icon::new(Logo::Claude)
                                .size(px(14.))
                                .text_color(gpui::rgb(0xd97757)),
                        )
                        .child("Claude")
                        .child(Icon::new(IconName::ChevronDown).size(px(12.)).opacity(0.7)),
                )
                .child(div().mx_0p5().h_4().w(px(1.)).bg(theme.border))
                .child(
                    pill("approval-mode")
                        .child(Icon::new(IconName::ShieldOff).size(px(14.)))
                        .child("Full access"),
                )
                .child(div().flex_1())
                .child(
                    div()
                        .id("send")
                        .flex()
                        .items_center()
                        .justify_center()
                        .size_8()
                        .rounded_full()
                        .bg(theme.primary)
                        .text_color(theme.primary_foreground)
                        // Nothing to send yet: disabled:opacity-30.
                        .opacity(0.3)
                        .child(Icon::new(IconName::ArrowUp).size(px(16.))),
                ),
        )
}
