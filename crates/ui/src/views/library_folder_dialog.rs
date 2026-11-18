//! LibraryFolderDialog.svelte in Dialog.svelte's frame: adding a folder to a library, or editing
//! one. A form edits a copy; the folder it was opened with is only its starting values. It can
//! be saved once the path is a folder on the server's computer.
//!
//! The folder is picked with the folder picker (FolderPicker.svelte): the server's computer's
//! folders listed as you type, and the system's folder dialog when the server is on this computer.

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, ParentElement, Render, StatefulInteractiveElement, Styled, Subscription, Window,
    div, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::Select;
use linker_edit_protocol::{LIBRARY_KINDS, LibraryFolder, LibraryFolderFields, LibraryKind};

use crate::browse_path::trim_separators;
use crate::library::kind_label;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::choice_select::{Choice, ChoiceState, choice_state, chosen};
use crate::views::dialog::{Dialog, primary_button};
use crate::views::folder_picker::{FolderPicker, FolderPickerEvent};
use crate::views::settings_parts::ghost_button;

pub enum FolderDialogEvent {
    Save(LibraryFolderFields),
    Close,
}

pub struct LibraryFolderDialog {
    editing: bool,
    /// Which library, as the dialog says it: "the universal library".
    where_: String,
    kind: Entity<ChoiceState>,
    path: Entity<FolderPicker>,
    label: Entity<InputState>,
    note: Entity<TextareaState>,
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
            let mut picker = FolderPicker::new(state.clone(), "Library folder", true, window, cx);
            if let Some(folder) = &folder {
                picker.set_path(folder.path.clone(), window, cx);
            }
            picker
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
        path.update(cx, |picker, cx| picker.focus(window, cx));
        let subscriptions = vec![
            cx.subscribe_in(
                &path,
                window,
                |this, _, event: &FolderPickerEvent, window, cx| match event {
                    FolderPickerEvent::Changed => cx.notify(),
                    FolderPickerEvent::Submit(_) => this.save(window, cx),
                },
            ),
            cx.subscribe(&label, |_, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify()
                }
            }),
        ];
        LibraryFolderDialog {
            editing: folder.is_some(),
            where_,
            kind,
            path,
            label,
            note,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    fn picked(&self, cx: &App) -> String {
        trim_separators(&self.path.read(cx).value(cx))
    }

    /// The folder's own name, the name a folder gets when none is typed.
    fn fallback_label(&self, cx: &App) -> String {
        self.picked(cx)
            .split(['\\', '/'])
            .rfind(|part| !part.is_empty())
            .unwrap_or_default()
            .to_string()
    }

    pub fn can_save(&self, cx: &App) -> bool {
        !self.picked(cx).is_empty()
            && self
                .path
                .read(cx)
                .status(cx)
                .is_some_and(|status| status.exists)
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

impl Render for LibraryFolderDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let can_save = self.can_save(cx);
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
            field("Folder", self.path.clone().into_any_element()),
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
                    primary_button(
                        "save-folder",
                        if self.editing { "Save" } else { "Add folder" },
                        !can_save,
                        &theme,
                        cx.listener(|this, _, window, cx| this.save(window, cx)),
                    )
                    .test_support(),
                )
                .into_any_element(),
        ];
        let this = cx.entity().downgrade();
        Dialog::new(
            "library-folder-dialog",
            if self.editing {
                "Edit folder"
            } else {
                "Add a folder"
            },
            &self.focus,
            move |_, cx| {
                let _ = this.update(cx, |_, cx| cx.emit(FolderDialogEvent::Close));
            },
        )
        .children(body)
    }
}
