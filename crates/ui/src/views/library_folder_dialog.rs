//! LibraryFolderDialog.svelte in Dialog.svelte's frame: adding a folder to a library, or editing
//! one. A form edits a copy; the folder it was opened with is only its starting values. It can
//! be saved once the path is a folder on the server's computer.
//!
//! The folder field is a path typed for the server's computer, checked as it is typed, with the
//! system's folder dialog when the server is on this computer. FolderPicker.svelte's listing of
//! the folders as you type comes with the add-project dialog's port; this field is where it goes.

use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, PathPromptOptions, Render,
    StatefulInteractiveElement, Styled, Subscription, Task, Window, div, prelude::FluentBuilder,
    px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use linker_edit_protocol::{
    LIBRARY_KINDS, LibraryFolder, LibraryFolderFields, LibraryKind, OrError,
};

use crate::icons::IconName;
use crate::library::kind_label;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::choice_select::{Choice, ChoiceState, choice_state, chosen};
use crate::views::settings_parts::{disable, ghost_button, primary_button};
use crate::views::thread_starter::trim_separators;

/// How long typing waits before the path is checked (FolderPicker's DEBOUNCE_MS).
const CHECK_AFTER: Duration = Duration::from_millis(100);

pub enum FolderDialogEvent {
    Save(LibraryFolderFields),
    Close,
}

/// What the path field holds: a folder that exists, or not; None while it is being checked.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathStatus {
    Checking,
    Folder,
    NotAFolder,
}

pub struct LibraryFolderDialog {
    state: Entity<AppState>,
    editing: bool,
    /// Which library, as the dialog says it: "the universal library".
    where_: String,
    kind: Entity<ChoiceState>,
    path: Entity<InputState>,
    label: Entity<InputState>,
    note: Entity<TextareaState>,
    status: PathStatus,
    check: Option<Task<()>>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FolderDialogEvent> for LibraryFolderDialog {}

impl Focusable for LibraryFolderDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

fn kind_value(kind: LibraryKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(String::from))
        .unwrap_or_default()
}

impl LibraryFolderDialog {
    pub fn new(
        state: Entity<AppState>,
        folder: Option<LibraryFolder>,
        where_: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let kinds = LIBRARY_KINDS
            .iter()
            .map(|&kind| Choice::new(kind_value(kind), kind_label(kind)))
            .collect();
        let kind = choice_state(
            kinds,
            &kind_value(folder.as_ref().map_or(LibraryKind::Music, |f| f.kind)),
            window,
            cx,
        );
        let path = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(folder.as_ref().map(|f| f.path.clone()).unwrap_or_default())
        });
        let label = cx.new(|cx| {
            InputState::new(window, cx)
                .default_value(folder.as_ref().map(|f| f.label.clone()).unwrap_or_default())
        });
        let note = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(3, 8)
                .placeholder("Licensed for YouTube only. Use for intros.")
                .default_value(folder.as_ref().map(|f| f.note.clone()).unwrap_or_default())
        });
        path.update(cx, |input, cx| input.focus(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &path,
                window,
                |this, _, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => this.check_path(cx),
                    InputEvent::PressEnter { .. } => this.save(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe(&label, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify()
                }
            }),
        ];
        let mut dialog = LibraryFolderDialog {
            state,
            editing: folder.is_some(),
            where_,
            kind,
            path,
            label,
            note,
            status: PathStatus::NotAFolder,
            check: None,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        dialog.check_path(cx);
        dialog
    }

    fn picked(&self, cx: &App) -> String {
        trim_separators(&self.path.read(cx).value())
    }

    /// The folder's own name, the name a folder gets when none is typed.
    fn fallback_label(&self, cx: &App) -> String {
        self.picked(cx)
            .split(['\\', '/'])
            .rfind(|part| !part.is_empty())
            .unwrap_or_default()
            .to_string()
    }

    pub fn status(&self) -> PathStatus {
        self.status
    }

    pub fn can_save(&self, cx: &App) -> bool {
        !self.picked(cx).is_empty() && self.status == PathStatus::Folder
    }

    fn check_path(&mut self, cx: &mut Context<Self>) {
        let picked = self.picked(cx);
        if picked.is_empty() {
            self.status = PathStatus::NotAFolder;
            self.check = None;
            cx.notify();
            return;
        }
        self.status = PathStatus::Checking;
        let state = self.state.clone();
        self.check = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(CHECK_AFTER).await;
            let answer = state.update(cx, |state, _| state.browse_folders(picked));
            let listing = answer.await;
            let _ = this.update(cx, |this, cx| {
                this.status = match listing {
                    OrError::Ok(_) => PathStatus::Folder,
                    OrError::Error { .. } => PathStatus::NotAFolder,
                };
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let answer = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            if let Ok(Ok(Some(paths))) = answer.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().to_string();
                let _ = this.update_in(cx, |this, window, cx| {
                    this.path
                        .update(cx, |input, cx| input.set_value(path, window, cx));
                    this.check_path(cx);
                });
            }
        })
        .detach();
    }

    fn save(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        if !self.can_save(cx) {
            return;
        }
        let kind: LibraryKind =
            serde_json::from_value(serde_json::Value::String(chosen(&self.kind, cx)))
                .unwrap_or(LibraryKind::Music);
        let label = self.label.read(cx).value().trim().to_string();
        cx.emit(FolderDialogEvent::Save(LibraryFolderFields {
            kind,
            path: self.picked(cx),
            label: if label.is_empty() {
                self.fallback_label(cx)
            } else {
                label
            },
            note: self.note.read(cx).value().trim().to_string(),
        }));
        cx.emit(FolderDialogEvent::Close);
    }
}

/// Dialog.svelte's frame: a card over a dimmed window, with its title and a close button. A
/// click on the dim closes it; Escape does too (the workspace's Back action).
pub fn dialog_frame(
    title: &'static str,
    body: Vec<AnyElement>,
    on_close: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::of(cx);
    let (accent, foreground) = (theme.accent, theme.foreground);
    let on_close = std::rc::Rc::new(on_close);
    let (close_backdrop, close_button) = (on_close.clone(), on_close);
    div()
        .id("dialog-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(gpui::black().opacity(0.4))
        .occlude()
        .on_click(move |_, window, cx| close_backdrop(window, cx))
        .child(
            div()
                .id("dialog")
                .w(px(480.))
                .rounded(theme.radius.xl)
                .border_1()
                .border_color(theme.border)
                .bg(theme.card)
                .text_color(theme.foreground)
                .shadow_xl()
                // Clicks inside stay inside.
                .on_click(|_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h_12()
                        .px_4()
                        .border_b_1()
                        .border_color(theme.border)
                        .child(
                            div()
                                .text_token(TextSize::Sm)
                                .font_weight(FontWeight::MEDIUM)
                                .child(title),
                        )
                        .child(
                            div()
                                .id("close-dialog")
                                .ml_auto()
                                .flex()
                                .items_center()
                                .justify_center()
                                .size_7()
                                .rounded(theme.radius.md)
                                .text_color(theme.muted_foreground)
                                .cursor_pointer()
                                .hover(move |style| style.bg(accent).text_color(foreground))
                                .on_click(move |_, window, cx| close_button(window, cx))
                                .child(Icon::new(IconName::X).size(px(16.)))
                                .test_support(),
                        ),
                )
                .child(div().flex().flex_col().gap_4().p_4().children(body))
                .test_support(),
        )
}

impl Render for LibraryFolderDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let can_save = self.can_save(cx);
        let on_this_computer = self.state.read(cx).on_this_computer();
        let fallback = self.fallback_label(cx);
        self.label.update(cx, |input, cx| {
            input.set_placeholder(
                if fallback.is_empty() {
                    "Epidemic downloads".to_string()
                } else {
                    fallback
                },
                window,
                cx,
            )
        });
        let field = |label: &'static str, control: AnyElement| {
            div()
                .flex()
                .flex_col()
                .gap_1()
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child(label)
                .child(control)
                .into_any_element()
        };
        let (accent, foreground) = (theme.accent, theme.foreground);
        let folder = div()
            .flex()
            .gap_1p5()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .font_family(theme.mono_font_family)
                    .child(Input::new(&self.path)),
            )
            .when(on_this_computer, |this| {
                this.child(
                    div()
                        .id("browse-folder")
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .gap_1p5()
                        .h_8()
                        .px_2p5()
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(theme.border)
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .cursor_pointer()
                        .hover(move |style| style.bg(accent).text_color(foreground))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Choose with the system's folder dialog",
                            )
                            .build(window, cx)
                        })
                        .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx)))
                        .child(Icon::new(IconName::FolderSearch).size(px(14.)))
                        .child("Browse…")
                        .test_support(),
                )
            })
            .into_any_element();
        let body = vec![
            div()
                .mt(px(-4.))
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child(format!(
                    "{} {}. Linker-Edit only points at the folder: your files stay where they are.",
                    if self.editing { "In" } else { "To" },
                    self.where_
                ))
                .into_any_element(),
            field("Kind", Select::new(&self.kind).into_any_element()),
            field("Folder", folder),
            field("Name", Input::new(&self.label).into_any_element()),
            field(
                "Note for you and the agents",
                div()
                    .rounded(theme.radius.md)
                    .border_1()
                    .border_color(theme.border)
                    .px_2p5()
                    .py_1p5()
                    .text_color(theme.foreground)
                    .child(Textarea::new(&self.note).appearance(false))
                    .into_any_element(),
            ),
            div()
                .flex()
                .items_center()
                .justify_end()
                .gap_2()
                .border_t_1()
                .border_color(theme.border)
                .pt_4()
                .child(
                    ghost_button("cancel-folder", &theme)
                        .text_token(TextSize::Sm)
                        .text_color(theme.foreground)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(FolderDialogEvent::Close)))
                        .child("Cancel")
                        .test_support(),
                )
                .child(
                    disable(primary_button("save-folder", &theme), !can_save)
                        .text_token(TextSize::Sm)
                        .when(can_save, |this| {
                            this.on_click(cx.listener(|this, _, window, cx| this.save(window, cx)))
                        })
                        .child(if self.editing { "Save" } else { "Add folder" })
                        .test_support(),
                )
                .into_any_element(),
        ];
        let this = cx.entity().downgrade();
        dialog_frame(
            if self.editing {
                "Edit folder"
            } else {
                "Add a folder"
            },
            body,
            move |_, cx| {
                let _ = this.update(cx, |_, cx| cx.emit(FolderDialogEvent::Close));
            },
            cx,
        )
        .track_focus(&self.focus)
        .test_support()
    }
}
