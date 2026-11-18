//! ToolRow.svelte: one tool call in a work group. Its icon, label and one-line summary; while it
//! runs "Running", a cross when it failed; a click shows its input and output.
//! Whether it is open is the chat's to keep (a row is drawn fresh each frame), so it takes it.

use gpui::{
    App, ClickEvent, ElementId, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, relative,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::ToolStatus;
use serde_json::Value;

use crate::components::OnClick;
use crate::icons::IconName;
use crate::labels::{format_json, is_resolve_tool, tool_label, tool_summary};
use crate::theme::{TextSize, TextStyled, Theme};

#[derive(IntoElement)]
pub struct ToolRow {
    id: ElementId,
    name: String,
    input: Value,
    status: ToolStatus,
    output: Option<String>,
    expanded: bool,
    on_toggle: OnClick,
}

impl ToolRow {
    pub fn new(
        id: &str,
        name: &str,
        input: &Value,
        status: ToolStatus,
        output: Option<&str>,
        expanded: bool,
        on_toggle: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        ToolRow {
            id: SharedString::from(format!("tool-{id}")).into(),
            name: name.to_string(),
            input: input.clone(),
            status,
            output: output.map(str::to_string),
            expanded,
            on_toggle: Box::new(on_toggle),
        }
    }
}

/// The icon for a tool, as ToolRow.svelte picks it.
pub fn tool_icon(name: &str) -> IconName {
    if is_resolve_tool(name) {
        return IconName::Clapperboard;
    }
    match name {
        "shell" | "Bash" => IconName::SquareTerminal,
        "Read" => IconName::FileText,
        "Edit" | "Write" | "edit files" => IconName::FilePen,
        "Grep" | "Glob" => IconName::Search,
        "WebSearch" | "WebFetch" | "web search" => IconName::Globe,
        "TodoWrite" => IconName::ListTodo,
        "ToolSearch" => IconName::BookOpen,
        _ => IconName::Wrench,
    }
}

impl RenderOnce for ToolRow {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let error = self.status == ToolStatus::Error;
        let summary = tool_summary(&self.name, &self.input);
        let pre = |text: String, theme: &Theme| {
            div()
                .w_full()
                .font_family(theme.mono_font_family)
                .text_size(TextSize::X2s.size())
                .line_height(relative(1.625))
                .text_color(theme.muted_foreground)
                .child(text)
        };
        let label_color: Hsla = if error {
            theme.destructive
        } else {
            theme.foreground.opacity(0.8)
        };

        div()
            .w_full()
            .min_w_0()
            .rounded(theme.radius.md)
            .px_0p5()
            .py_0p5()
            .child(
                div()
                    .id(self.id.clone())
                    .flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .rounded(theme.radius.md)
                    .text_size(TextSize::Sm.size())
                    .line_height(relative(1.625))
                    .cursor_pointer()
                    .on_click(self.on_toggle)
                    .child(
                        div()
                            .flex()
                            .size_6()
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .text_color(if error {
                                theme.destructive
                            } else {
                                theme.muted_foreground
                            })
                            .child(Icon::new(tool_icon(&self.name)).size(px(16.))),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_baseline()
                            .gap_1p5()
                            .overflow_hidden()
                            .child(
                                div()
                                    .flex_shrink_0()
                                    .text_color(label_color)
                                    .when(error, |this| this.font_weight(FontWeight::MEDIUM))
                                    .child(tool_label(&self.name).to_string()),
                            )
                            .when(!summary.is_empty(), |this| {
                                this.child(
                                    div()
                                        .min_w_0()
                                        .truncate()
                                        .text_color(theme.muted_foreground)
                                        .child(summary),
                                )
                            }),
                    )
                    .when(self.status == ToolStatus::Running, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_token(TextSize::Xs)
                                .text_color(theme.muted_foreground)
                                .child("Running"),
                        )
                    })
                    .when(error, |this| {
                        this.child(
                            Icon::new(IconName::X)
                                .size(px(12.))
                                .text_color(theme.destructive),
                        )
                    })
                    .child(
                        Icon::new(if self.expanded {
                            IconName::ChevronDown
                        } else {
                            IconName::ChevronRight
                        })
                        .size(px(12.))
                        .text_color(theme.muted_foreground.opacity(0.7)),
                    )
                    .test_support(),
            )
            .when(self.expanded, |this| {
                this.child(
                    div()
                        .id(SharedString::from(format!("{}-detail", self.id)))
                        .ml_7()
                        .flex()
                        .flex_col()
                        .gap_2()
                        .max_h(px(384.))
                        .overflow_y_scroll()
                        .px_0p5()
                        .py_1()
                        .child(pre(format_json(&self.input), theme))
                        .when_some(self.output, |this, output| {
                            this.when(!output.is_empty(), |this| {
                                this.child(
                                    pre(output, theme)
                                        .border_l_2()
                                        .border_color(theme.border)
                                        .pl_2p5(),
                                )
                            })
                        })
                        .test_support(),
                )
            })
    }
}
