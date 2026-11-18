//! ApprovalCard.svelte: a tool waiting for the user's OK, above the composer. A change to a
//! workspace's or series' brief reads as what it is: new rules against the old, or a lesson
//! next to the user's words; anything else shows its script or input.

use std::rc::Rc;

use gpui::{
    App, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::ApprovalRequest;

use crate::icons::IconName;
use crate::labels::{
    BriefRequest, LineKind, approval_detail, brief_request, is_resolve_tool, line_diff, tool_label,
};
use crate::theme::{TextSize, TextStyled, Theme};

type OnAnswer = Rc<dyn Fn(bool, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct ApprovalCard {
    request: ApprovalRequest,
    on_answer: OnAnswer,
}

impl ApprovalCard {
    pub fn new(
        request: ApprovalRequest,
        on_answer: impl Fn(bool, &mut Window, &mut App) + 'static,
    ) -> Self {
        ApprovalCard {
            request,
            on_answer: Rc::new(on_answer),
        }
    }
}

/// The composer's shadow (`shadow-composer`), light theme only.
pub fn composer_shadow(theme: &Theme) -> Vec<gpui::BoxShadow> {
    if theme.dark {
        return Vec::new();
    }
    vec![gpui::BoxShadow {
        color: gpui::hsla(0., 0., 0., 0.05),
        offset: gpui::point(px(0.), px(1.)),
        blur_radius: px(2.),
        spread_radius: px(0.),
        inset: false,
    }]
}

impl RenderOnce for ApprovalCard {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let request = &self.request;
        let brief = brief_request(&request.tool, &request.input);
        let id: ElementId = SharedString::from(format!("approval-{}", request.id)).into();

        let heading = |icon: IconName, children: Vec<gpui::AnyElement>| {
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_token(TextSize::Sm)
                .child(Icon::new(icon).size(px(16.)).text_color(theme.warning))
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_w_0()
                        .items_baseline()
                        .gap_1()
                        .overflow_hidden()
                        .children(children),
                )
        };
        let caption = |text: &'static str| {
            div()
                .text_token(TextSize::X2s)
                .text_color(theme.muted_foreground)
                .child(text)
        };

        let body = match &brief {
            Some(BriefRequest::Lesson {
                for_,
                feedback,
                rule,
            }) => div()
                .child(heading(
                    IconName::GraduationCap,
                    vec![
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::MEDIUM)
                            .child("New lesson for")
                            .into_any_element(),
                        div().truncate().child(for_.clone()).into_any_element(),
                    ],
                ))
                .child(
                    div()
                        .mt_2()
                        .ml_6()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .text_token(TextSize::Sm)
                        .child(
                            div().child(caption("From what you said")).child(
                                div()
                                    .border_l_2()
                                    .border_color(theme.border)
                                    .pl_2()
                                    .italic()
                                    .text_color(theme.muted_foreground)
                                    .child(feedback.clone()),
                            ),
                        )
                        .child(
                            div()
                                .child(caption("The agent will follow this from now on"))
                                .child(div().child(rule.clone())),
                        ),
                ),
            Some(BriefRequest::Rules {
                for_,
                summary,
                was,
                rules,
            }) => {
                let title = if !was.is_empty() {
                    "Change the rules of"
                } else {
                    "Rules for"
                };
                let lines = line_diff(was, rules).into_iter().map(|line| {
                    let (mark, row) = match line.kind {
                        LineKind::Same => (" ", div().text_color(theme.muted_foreground)),
                        LineKind::Added => (
                            "+",
                            div()
                                .bg(theme.success.opacity(0.1))
                                .text_color(theme.foreground),
                        ),
                        LineKind::Removed => (
                            "−",
                            div()
                                .bg(theme.destructive.opacity(0.1))
                                .text_color(theme.muted_foreground)
                                .line_through(),
                        ),
                    };
                    let text = if line.text.is_empty() {
                        " ".to_string()
                    } else {
                        line.text
                    };
                    row.flex()
                        .gap_2()
                        .px_2()
                        .child(div().flex_shrink_0().child(mark))
                        .child(div().min_w_0().child(text))
                });
                div()
                    .child(heading(
                        IconName::GraduationCap,
                        vec![
                            div()
                                .flex_shrink_0()
                                .font_weight(FontWeight::MEDIUM)
                                .child(title)
                                .into_any_element(),
                            div().truncate().child(for_.clone()).into_any_element(),
                        ],
                    ))
                    .when(!summary.is_empty(), |this| {
                        this.child(
                            div()
                                .mt_2()
                                .ml_6()
                                .text_token(TextSize::Sm)
                                .text_color(theme.muted_foreground)
                                .child(summary.clone()),
                        )
                    })
                    .child(
                        div()
                            .id("rules-diff")
                            .mt_2()
                            .ml_6()
                            .max_h(px(288.))
                            .overflow_y_scroll()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(theme.border)
                            .font_family(theme.mono_font_family)
                            .text_size(TextSize::Xs.size())
                            .line_height(px(20.))
                            .children(lines),
                    )
            }
            None => div()
                .child(heading(
                    IconName::ShieldAlert,
                    vec![
                        div()
                            .flex_shrink_0()
                            .font_weight(FontWeight::MEDIUM)
                            .child(if is_resolve_tool(&request.tool) {
                                "Allow Resolve:"
                            } else {
                                "Allow tool:"
                            })
                            .into_any_element(),
                        div()
                            .truncate()
                            .font_family(theme.mono_font_family)
                            .text_size(TextSize::Xs.size())
                            .text_color(theme.foreground.opacity(0.8))
                            .child(tool_label(&request.tool).to_string())
                            .into_any_element(),
                    ],
                ))
                .child(
                    div()
                        .id("approval-detail")
                        .mt_2()
                        .ml_6()
                        .max_h(px(256.))
                        .overflow_y_scroll()
                        .font_family(theme.mono_font_family)
                        .text_size(TextSize::Xs.size())
                        .line_height(px(20.))
                        .text_color(theme.muted_foreground)
                        .child(approval_detail(&request.tool, &request.input)),
                ),
        };

        let footnote = brief.as_ref().map(|brief| match brief {
            BriefRequest::Lesson { .. } => {
                "Decline it if this isn't what you meant. You can remove a lesson later in Settings."
            }
            BriefRequest::Rules { .. } => {
                "Every edit in it follows these. You can change them later in Settings."
            }
        });
        let (no, yes) = if brief.is_some() {
            ("Decline", "Accept")
        } else {
            ("Deny", "Allow")
        };
        let answer = |allow: bool| {
            let on_answer = self.on_answer.clone();
            move |_: &gpui::ClickEvent, window: &mut Window, cx: &mut App| {
                on_answer(allow, window, cx)
            }
        };

        div()
            .id(id)
            .rounded(theme.radius.xl2)
            .border_1()
            .border_color(theme.warning.opacity(0.3))
            .bg(theme.card)
            .px_4()
            .py_3()
            .shadow(composer_shadow(&theme))
            .child(body)
            .when_some(footnote, |this, footnote| {
                this.child(
                    div()
                        .mt_2()
                        .ml_6()
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .child(footnote),
                )
            })
            .child(
                div()
                    .mt_3()
                    .flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        div()
                            .id("deny")
                            .flex()
                            .items_center()
                            .h_7()
                            .px_3()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(theme.border)
                            .text_token(TextSize::Xs)
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.accent))
                            .on_click(answer(false))
                            .child(no)
                            .test_support(),
                    )
                    .child(
                        div()
                            .id("allow")
                            .flex()
                            .items_center()
                            .h_7()
                            .px_3()
                            .rounded(theme.radius.md)
                            .bg(theme.primary)
                            .text_color(theme.primary_foreground)
                            .text_token(TextSize::Xs)
                            .font_weight(FontWeight::MEDIUM)
                            .cursor_pointer()
                            .hover(|style| style.opacity(0.9))
                            .on_click(answer(true))
                            .child(yes)
                            .test_support(),
                    ),
            )
            .test_support()
    }
}
