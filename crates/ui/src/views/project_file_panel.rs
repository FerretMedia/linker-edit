//! ProjectFilePanel.svelte: PROJECT.md of the project in view, to read and edit. The text follows
//! the file on disk (the agent writes it in its turns) until the user types; then it is their
//! draft until they save it (Save or Ctrl+S) or undo it. When the agent changes the file under a
//! draft, it says so. The panel asks for the file while open, after a reconnect and a switch of
//! project; the server pushes it again after each turn and each save. The workspace makes a new
//! one per project, as `{#key app.projectId}` does.

use gpui::{
    AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, relative,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{Sizable, Size};
use linker_edit_protocol::ClientMessage;

use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::panel::{Panel, close_button, panel_frame, panel_header};
use crate::views::settings_parts::{disable, outline_button, primary_button};

pub enum ProjectFileEvent {
    Close,
}

const PLACEHOLDER: &str = "Nothing yet. The agent writes it when it learns what the video is, or start it yourself:\n\n# Working title\n\n## Brief\nWhat the video is about, who it's for, what must be in it.";

pub struct ProjectFilePanel {
    state: Entity<AppState>,
    input: Entity<TextareaState>,
    /// What the user typed since the text on disk last changed; None follows the disk.
    draft: Option<String>,
    /// The text on disk when the draft began.
    basis: Option<String>,
    /// The text on disk the input shows while there is no draft.
    shown: Option<String>,
    /// Set while the input is changed from here, so it isn't taken for typing.
    syncing: bool,
    asked: Option<(bool, Option<String>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ProjectFileEvent> for ProjectFilePanel {}

impl ProjectFilePanel {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| TextareaState::new(window, cx).placeholder(PLACEHOLDER));
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| {
                this.follow(window, cx)
            }),
            cx.subscribe(&input, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) && !this.syncing {
                    let value = input.read(cx).value().to_string();
                    this.edit(value, cx);
                }
            }),
        ];
        let mut panel = ProjectFilePanel {
            state,
            input,
            draft: None,
            basis: None,
            shown: None,
            syncing: false,
            asked: None,
            _subscriptions: subscriptions,
        };
        panel.follow(window, cx);
        panel
    }

    /// The text on disk; None while it loads.
    fn saved(&self, cx: &gpui::App) -> Option<String> {
        self.state
            .read(cx)
            .project_file_in_view()
            .map(|file| file.text.clone())
    }

    pub fn changed(&self, cx: &gpui::App) -> bool {
        self.draft
            .as_ref()
            .is_some_and(|draft| Some(draft) != self.saved(cx).as_ref())
    }

    /// The file changed on disk under an edit: the agent wrote it in a turn.
    pub fn overtaken(&self, cx: &gpui::App) -> bool {
        self.changed(cx) && self.basis.is_some() && self.basis != self.saved(cx)
    }

    fn follow(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let now = (state.connected, state.project_id().map(String::from));
        if self.asked.as_ref() != Some(&now) {
            if now.0 && now.1.is_some() {
                state.send(ClientMessage::LoadProjectFile);
            }
            self.asked = Some(now);
        }
        let saved = self.saved(cx);
        if self.draft.is_none() && saved != self.shown {
            self.show(saved.clone().unwrap_or_default(), window, cx);
            self.shown = saved;
        }
        cx.notify();
    }

    fn show(&mut self, text: String, window: &mut Window, cx: &mut Context<Self>) {
        self.syncing = true;
        self.input
            .update(cx, |input, cx| input.set_value(text, window, cx));
        self.syncing = false;
    }

    fn edit(&mut self, value: String, cx: &mut Context<Self>) {
        if self.draft.is_none() {
            self.basis = self.saved(cx);
        }
        self.draft = Some(value);
        cx.notify();
    }

    fn reset(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.draft = None;
        self.basis = None;
        let saved = self.saved(cx);
        self.show(saved.clone().unwrap_or_default(), window, cx);
        self.shown = saved;
        cx.notify();
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.changed(cx) {
            return;
        }
        let state = self.state.read(cx);
        if let (Some(project_id), Some(text)) = (state.project_id(), &self.draft) {
            state.send(ClientMessage::SaveProjectFile {
                project_id: project_id.into(),
                text: text.clone(),
            });
        }
        // It follows the disk again, which the server sends anew after the save.
        self.reset(window, cx);
    }
}

impl Render for ProjectFilePanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let file = self.state.read(cx).project_file_in_view().cloned();
        let changed = self.changed(cx);
        let overtaken = self.overtaken(cx);

        let header = panel_header(Panel::File, "Project file", window, &theme).child(close_button(
            "Close project file",
            &theme,
            cx.listener(|_, _, _, cx| cx.emit(ProjectFileEvent::Close)),
        ));

        let body = match file {
            None => div()
                .id("project-file-loading")
                .px_4()
                .py_6()
                .text_center()
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child("Loading the project file…")
                .test_support()
                .into_any_element(),
            Some(file) => div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .gap_2()
                .p_4()
                .child(
                    div()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child("What this video is: its brief, what is in the footage, decisions, your feedback and where it stands. The agent keeps it and reads it in every thread of this project. You can change it too."),
                )
                .child(
                    div()
                        .id("project-file-text")
                        .flex_1()
                        .min_h_0()
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(theme.border)
                        .px_2p5()
                        .py_2()
                        .font_family(theme.mono_font_family)
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            let keys = &event.keystroke;
                            if keys.key == "s" && (keys.modifiers.control || keys.modifiers.platform) {
                                cx.stop_propagation();
                                this.save(window, cx);
                            }
                        }))
                        .child(
                            Textarea::new(&self.input)
                                .appearance(false)
                                .h_full()
                                .with_size(Size::XSmall)
                                .text_size(TextSize::Xs.size())
                                .line_height(relative(1.625)),
                        )
                        .test_support(),
                )
                .when(overtaken, |this| {
                    this.child(
                        div()
                            .id("project-file-overtaken")
                            .rounded(theme.radius.lg)
                            .border_1()
                            .border_color(theme.warning.opacity(0.3))
                            .bg(theme.warning.opacity(0.1))
                            .px_3()
                            .py_2()
                            .text_token(TextSize::Xs)
                            .text_color(theme.warning_foreground)
                            .child("The agent changed this file while you were editing. Saving replaces its version; Undo shows it.")
                            .test_support(),
                    )
                })
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .truncate()
                                .font_family(theme.mono_font_family)
                                .text_token(TextSize::X2s)
                                .text_color(theme.muted_foreground)
                                .child(file.file.clone()),
                        )
                        .when(changed, |this| {
                            this.child(
                                outline_button("undo-project-file", &theme)
                                    .on_click(cx.listener(|this, _, window, cx| this.reset(window, cx)))
                                    .child("Undo")
                                    .test_support(),
                            )
                        })
                        .child(
                            disable(primary_button("save-project-file", &theme), !changed)
                                .when(changed, |this| {
                                    this.on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
                                })
                                .child(if changed { "Save" } else { "Saved" })
                                .test_support(),
                        ),
                )
                .into_any_element(),
        };

        panel_frame(Panel::File, &theme)
            .child(header)
            .child(body)
            .test_support()
    }
}
