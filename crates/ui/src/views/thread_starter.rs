//! ThreadStarter.svelte: how an empty thread starts when the agent needs folders rather than a
//! question: a setup thread's example videos, or the footage of a video it edits on its own. It
//! writes the first message for the user, as +page.svelte's `start` does.
//!
//! The folders are typed paths on the server's computer. FolderPicker.svelte's listing of the
//! folders as you type is the add-project dialog's too, and comes with it.

use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder,
    px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use linker_edit_protocol::ClientMessage;

use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};

/// Which starter, for which workspace or series.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Starter {
    /// A workspace's or series' setup thread: teach the agent how it is edited.
    Setup { collection: String },
    /// A video in a workspace or series whose agent makes the first cut on its own.
    Autonomous { collection: String },
}

struct Field {
    label: &'static str,
    hint: &'static str,
    required: bool,
    input: Entity<InputState>,
}

pub struct ThreadStarter {
    state: Entity<AppState>,
    starter: Starter,
    fields: Vec<Field>,
    notes: Entity<TextareaState>,
    _subscriptions: Vec<Subscription>,
}

impl ThreadStarter {
    pub fn new(
        state: Entity<AppState>,
        starter: Starter,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let specs: &[(&str, &str, bool)] = match starter {
            Starter::Setup { .. } => &[
                (
                    "Finished videos",
                    "A folder of videos as they were published.",
                    true,
                ),
                (
                    "Raw footage",
                    "What some of them were cut from. It shows what an editor leaves out.",
                    false,
                ),
                ("Brand assets", "Logo, intro, music, LUTs.", false),
            ],
            Starter::Autonomous { .. } => {
                &[("Footage", "The folder with this video's footage.", true)]
            }
        };
        let mut subscriptions = Vec::new();
        let fields = specs
            .iter()
            .map(|(label, hint, required)| {
                let input = cx.new(|cx| InputState::new(window, cx));
                subscriptions.push(cx.subscribe(&input, |_, _, _: &InputEvent, cx| cx.notify()));
                Field {
                    label,
                    hint,
                    required: *required,
                    input,
                }
            })
            .collect();
        let placeholder = match starter {
            Starter::Setup { .. } => {
                "Which videos belong to which format, who watches them, where they're published…"
            }
            Starter::Autonomous { .. } => "The second interview is the important one.",
        };
        let notes = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 12)
                .placeholder(placeholder)
        });
        ThreadStarter {
            state,
            starter,
            fields,
            notes,
            _subscriptions: subscriptions,
        }
    }

    pub fn starter(&self) -> &Starter {
        &self.starter
    }

    fn folders(&self, cx: &gpui::App) -> Vec<String> {
        self.fields
            .iter()
            .map(|field| trim_separators(&field.input.read(cx).value()))
            .collect()
    }

    fn missing(&self, cx: &gpui::App) -> bool {
        self.fields
            .iter()
            .any(|field| field.required && field.input.read(cx).value().trim().is_empty())
    }

    /// The first message, from the folders picked and the notes.
    pub fn message(&self, cx: &gpui::App) -> String {
        let folders = self.folders(cx);
        let or_none = |folder: &String| {
            if folder.is_empty() {
                "none".to_string()
            } else {
                folder.clone()
            }
        };
        let mut lines = match &self.starter {
            Starter::Setup { collection } => vec![
                format!("Set up the brief for {collection}."),
                String::new(),
                format!("Finished videos: {}", folders[0]),
                format!("Raw footage: {}", or_none(&folders[1])),
                format!("Brand assets: {}", or_none(&folders[2])),
            ],
            Starter::Autonomous { .. } => vec![
                "Make the first cut of this video on your own.".into(),
                String::new(),
                format!("Footage: {}", folders[0]),
            ],
        };
        let notes = self.notes.read(cx).value().trim().to_string();
        if !notes.is_empty() {
            lines.push(String::new());
            lines.push(notes);
        }
        lines.join("\n")
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        if self.missing(cx) || self.state.read(cx).start_blocked().is_some() {
            return;
        }
        let text = self.message(cx);
        self.state.read(cx).send(ClientMessage::Send {
            text,
            compact_first: None,
        });
    }
}

impl Render for ThreadStarter {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let blocked = self.state.read(cx).start_blocked();
        let missing = self.missing(cx);
        let (title, intro, button) = match &self.starter {
            Starter::Setup { collection } => (
                format!("Teach Linker-Edit how {collection} is edited"),
                "Point at videos you've finished. The agent studies them, asks what it can't see, and proposes the rules for you to accept. It doesn't touch Resolve.".to_string(),
                "Start",
            ),
            Starter::Autonomous { collection } => (
                "New video".to_string(),
                format!("{collection} works on its own: give it the footage and it makes the first cut, then tells you what it decided. You give feedback on the video."),
                "Make the first cut",
            ),
        };
        let optional = |theme: &Theme| {
            div()
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child("· optional")
        };
        let fields = self.fields.iter().map(|field| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(
                    div()
                        .flex()
                        .items_baseline()
                        .gap_1()
                        .text_token(TextSize::Sm)
                        .child(field.label)
                        .when(!field.required, |this| this.child(optional(&theme))),
                )
                .child(Input::new(&field.input))
                .child(
                    div()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child(field.hint),
                )
        });
        let tip = blocked
            .clone()
            .or_else(|| missing.then(|| "Pick a folder first.".to_string()));
        let disabled = tip.is_some();

        div()
            .id("thread-starter")
            .my_auto()
            .flex()
            .flex_col()
            .gap_4()
            .w_full()
            .max_w(px(576.))
            .mx_auto()
            .py_10()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .items_center()
                    .child(
                        div()
                            .text_token(TextSize::X2l)
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .mt_2()
                            .text_token(TextSize::Sm)
                            .text_color(theme.muted_foreground)
                            .text_center()
                            .child(intro),
                    ),
            )
            .children(fields)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_token(TextSize::Sm)
                    .child(
                        div()
                            .flex()
                            .items_baseline()
                            .gap_1()
                            .child("Anything it should know")
                            .child(optional(&theme)),
                    )
                    .child(
                        div()
                            .rounded(theme.radius.md)
                            .border_1()
                            .border_color(theme.border)
                            .px_2p5()
                            .py_1p5()
                            .child(Textarea::new(&self.notes).appearance(false)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_3()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child("Or just write in the box below."),
                    )
                    .child(
                        div()
                            .id("start")
                            .flex()
                            .items_center()
                            .h_8()
                            .px_3()
                            .rounded(theme.radius.md)
                            .bg(theme.primary)
                            .text_color(theme.primary_foreground)
                            .text_token(TextSize::Sm)
                            .when_some(tip, |this, tip| {
                                this.tooltip(move |window, cx| {
                                    Tooltip::new(tip.clone()).build(window, cx)
                                })
                            })
                            .map(|this| {
                                if disabled {
                                    this.opacity(0.5)
                                } else {
                                    this.cursor_pointer()
                                        .hover(|style| style.opacity(0.9))
                                        .on_click(cx.listener(|this, _, _, cx| this.start(cx)))
                                }
                            })
                            .child(button)
                            .test_support(),
                    ),
            )
            .test_support()
    }
}

/// Without trailing separators, except for a root such as C:\ or /, which needs one
/// (browsePath.ts's trimSeparators).
pub fn trim_separators(value: &str) -> String {
    let trimmed = value.trim();
    let stripped = trimmed.trim_end_matches(['\\', '/']);
    if stripped.is_empty() {
        return trimmed.chars().next().map(String::from).unwrap_or_default();
    }
    let bytes = stripped.as_bytes();
    if bytes.len() == 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' {
        return format!("{stripped}\\");
    }
    stripped.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // browsePath.test.ts
    #[test]
    fn trims_trailing_separators_but_keeps_a_root() {
        assert_eq!(trim_separators("C:\\Users\\jarno\\"), "C:\\Users\\jarno");
        assert_eq!(trim_separators("C:\\"), "C:\\");
        assert_eq!(trim_separators("C:"), "C:\\");
        assert_eq!(trim_separators("/"), "/");
        assert_eq!(trim_separators(" /media/ "), "/media");
        assert_eq!(trim_separators(""), "");
    }
}
