//! FolderPicker.svelte: a path field that lists the folders of the server's computer as you type,
//! like T3 Code's "Add project": ↑/↓ choose, Enter or Tab goes into a folder, Enter on the path
//! itself (or Ctrl+Enter) picks it. The listing comes from the server ([`AppState::browse_folders`]),
//! so on another computer it is that computer's folders.
//!
//! "Browse…" opens the system's folder dialog, where the Electron window offered it: only on this
//! computer's own server, where a folder picked here means something.

use std::collections::HashMap;
use std::time::Duration;

use gpui::{
    AppContext, Context, Entity, EventEmitter, Focusable, InteractiveElement, IntoElement,
    ParentElement, PathPromptOptions, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, Task, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::base::input::{IndentInline, MoveDown, MoveUp};
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use linker_edit_protocol::{FolderEntry, FolderListing, OrError};

use crate::browse_path::{
    append_segment, browse_directory, browse_filter, filter_entries, has_trailing_separator,
    parent_directory, tab_completion, trim_separators,
};
use crate::icons::IconName;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::dialog::bare_input;

const DEBOUNCE: Duration = Duration::from_millis(100);

/// What's typed: an existing folder, and whether it holds a linker-edit.json.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FolderStatus {
    pub exists: bool,
    pub is_project: bool,
}

pub enum FolderPickerEvent {
    /// The path changed.
    Changed,
    /// Enter on the path itself, with no folder in the list chosen.
    Submit(String),
}

#[derive(Clone, Debug, PartialEq)]
enum Row {
    Up(String),
    Entry(FolderEntry),
}

pub struct FolderPicker {
    state: Entity<AppState>,
    input: Entity<InputState>,
    label: SharedString,
    /// Shows the list only while the field has focus, for a form with other fields.
    list_on_focus: bool,
    focused: bool,
    cache: HashMap<String, OrError<FolderListing>>,
    /// The last good listing, for the drive shortcuts while another folder loads.
    last_listing: Option<FolderListing>,
    highlight: Option<usize>,
    loading: Option<(String, Task<()>)>,
    started: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<FolderPickerEvent> for FolderPicker {}

impl FolderPicker {
    pub fn new(
        state: Entity<AppState>,
        label: impl Into<SharedString>,
        list_on_focus: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let input = cx.new(|cx| InputState::new(window, cx));
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => this.changed(cx),
                        InputEvent::PressEnter { secondary, .. } => {
                            this.enter(*secondary, window, cx)
                        }
                        InputEvent::Focus => {
                            this.focused = true;
                            this.start(window, cx);
                            cx.notify()
                        }
                        InputEvent::Blur => {
                            this.focused = false;
                            cx.notify()
                        }
                    }
                }),
            ];
        let mut picker = FolderPicker {
            state,
            input,
            label: label.into(),
            list_on_focus,
            focused: false,
            cache: HashMap::new(),
            last_listing: None,
            highlight: None,
            loading: None,
            started: false,
            _subscriptions: subscriptions,
        };
        picker.start(window, cx);
        picker
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Starts the field at `path` (a folder being edited), without moving the cursor into it.
    pub fn set_path(&mut self, path: String, window: &mut Window, cx: &mut Context<Self>) {
        self.input
            .update(cx, |input, cx| input.set_value(path, window, cx));
        self.changed(cx);
    }

    pub fn value(&self, cx: &gpui::App) -> String {
        self.input.read(cx).value().to_string()
    }

    fn show_list(&self) -> bool {
        !self.list_on_focus || self.focused
    }

    /// Starts an empty field in the home folder once the list shows, written out the way the
    /// server's computer writes paths. Not before: a field that only lists on focus shouldn't
    /// fill itself in.
    fn start(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.show_list() || self.started {
            return;
        }
        self.started = true;
        if !self.value(cx).trim().is_empty() {
            self.changed(cx);
            return;
        }
        let answer = self
            .state
            .update(cx, |state, _| state.browse_folders(String::new()));
        cx.spawn_in(window, async move |this, cx| {
            let result = answer.await;
            let _ = this.update_in(cx, |this, window, cx| {
                let dir = match &result {
                    OrError::Ok(listing) => Some(listing.dir.clone()),
                    OrError::Error { .. } => None,
                };
                this.store(String::new(), result);
                if let Some(dir) = dir
                    && this.value(cx).trim().is_empty()
                {
                    this.input
                        .update(cx, |input, cx| input.set_value(dir, window, cx));
                    this.changed(cx);
                }
            });
        })
        .detach();
    }

    fn store(&mut self, path: String, result: OrError<FolderListing>) {
        if let OrError::Ok(listing) = &result {
            self.last_listing = Some(listing.clone());
            // The folder as the server spells it, so going up from it works without asking again.
            if listing.dir != path {
                self.cache.insert(listing.dir.clone(), result.clone());
            }
        }
        self.cache.insert(path, result);
    }

    fn directory(&self, cx: &gpui::App) -> String {
        browse_directory(self.value(cx).trim())
    }

    fn filter(&self, cx: &gpui::App) -> String {
        browse_filter(self.value(cx).trim())
    }

    fn listing(&self, cx: &gpui::App) -> Option<&FolderListing> {
        match self.cache.get(&self.directory(cx))? {
            OrError::Ok(listing) => Some(listing),
            OrError::Error { .. } => None,
        }
    }

    fn rows(&self, cx: &gpui::App) -> Vec<Row> {
        let filter = self.filter(cx);
        let Some(listing) = self.listing(cx) else {
            return Vec::new();
        };
        let parent = filter
            .is_empty()
            .then(|| parent_directory(&listing.dir))
            .flatten();
        parent
            .map(Row::Up)
            .into_iter()
            .chain(
                filter_entries(&listing.entries, &filter)
                    .into_iter()
                    .cloned()
                    .map(Row::Entry),
            )
            .collect()
    }

    /// What's typed: an existing folder, and whether it's a project. None while it's being checked.
    pub fn status(&self, cx: &gpui::App) -> Option<FolderStatus> {
        let directory = self.directory(cx);
        let missing = FolderStatus {
            exists: false,
            is_project: false,
        };
        if directory.is_empty() {
            return Some(missing);
        }
        let listing = match self.cache.get(&directory)? {
            OrError::Ok(listing) => listing,
            OrError::Error { .. } => return Some(missing),
        };
        let filter = self.filter(cx);
        if filter.is_empty() {
            return Some(FolderStatus {
                exists: true,
                is_project: listing.is_project,
            });
        }
        let exact = listing
            .entries
            .iter()
            .find(|entry| entry.name.to_lowercase() == filter.to_lowercase());
        Some(FolderStatus {
            exists: exact.is_some(),
            is_project: exact.is_some_and(|entry| entry.is_project),
        })
    }

    /// The path changed: list its folder (after a pause), and choose a typed filter's first match.
    fn changed(&mut self, cx: &mut Context<Self>) {
        let directory = self.directory(cx);
        let filter = self.filter(cx);
        let rows = self.rows(cx);
        self.highlight = if filter.is_empty() {
            None
        } else {
            rows.iter().position(|row| matches!(row, Row::Entry(_)))
        };
        if !directory.is_empty()
            && !self.cache.contains_key(&directory)
            && self.loading.as_ref().map(|(path, _)| path) != Some(&directory)
        {
            let state = self.state.clone();
            let path = directory.clone();
            let task = cx.spawn(async move |this, cx| {
                cx.background_executor().timer(DEBOUNCE).await;
                let answer = state.update(cx, |state, _| state.browse_folders(path.clone()));
                let result = answer.await;
                let _ = this.update(cx, |this, cx| {
                    this.store(path, result);
                    this.loading = None;
                    this.changed(cx);
                });
            });
            self.loading = Some((directory, task));
        }
        cx.emit(FolderPickerEvent::Changed);
        cx.notify();
    }

    fn set_value(&mut self, next: String, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.replace_all(next, window, cx);
            input.focus(window, cx);
        });
        self.changed(cx);
    }

    fn activate(&mut self, row: Row, window: &mut Window, cx: &mut Context<Self>) {
        let next = match row {
            Row::Up(path) => path,
            Row::Entry(entry) => append_segment(self.value(cx).trim(), &entry.name),
        };
        self.set_value(next, window, cx);
    }

    fn enter(&mut self, secondary: bool, window: &mut Window, cx: &mut Context<Self>) {
        let row = self
            .highlight
            .and_then(|index| self.rows(cx).into_iter().nth(index));
        match row {
            Some(row) if !secondary => self.activate(row, window, cx),
            _ => {
                let value = self.value(cx);
                if !value.trim().is_empty() {
                    cx.emit(FolderPickerEvent::Submit(trim_separators(&value)));
                }
            }
        }
    }

    fn arrow(&mut self, down: bool, cx: &mut Context<Self>) {
        let count = self.rows(cx).len();
        if count == 0 {
            return;
        }
        self.highlight = match (self.highlight, down) {
            (None, true) => Some(0),
            (Some(at), true) => Some((at + 1).min(count - 1)),
            (Some(0) | None, false) => None,
            (Some(at), false) => Some(at - 1),
        };
        cx.notify();
    }

    /// Shell-style Tab: completes a typed name, else goes into the chosen folder. False when Tab
    /// has nothing to do here, so it moves the focus on.
    fn tab(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let filter = self.filter(cx);
        let completion = if filter.is_empty() {
            None
        } else {
            self.listing(cx).and_then(|listing| {
                tab_completion(&filter_entries(&listing.entries, &filter), &filter)
            })
        };
        if let Some(completion) = completion {
            let value = self.value(cx);
            let next = if completion.complete {
                append_segment(value.trim(), &completion.name)
            } else {
                format!("{}{}", self.directory(cx), completion.name)
            };
            self.set_value(next, window, cx);
            return true;
        }
        let row = self
            .highlight
            .and_then(|index| self.rows(cx).into_iter().nth(index));
        match row {
            Some(row) => {
                self.activate(row, window, cx);
                true
            }
            None => false,
        }
    }

    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let picked = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = picked.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let picked = path.to_string_lossy().to_string();
            let _ = this.update_in(cx, |this, window, cx| {
                let next = if has_trailing_separator(&picked) {
                    picked
                } else {
                    let separator = if picked.contains('\\') { '\\' } else { '/' };
                    format!("{picked}{separator}")
                };
                this.set_value(next, window, cx);
            });
        })
        .detach();
    }
}

impl Render for FolderPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let can_pick = self.state.read(cx).on_this_computer();
        let directory = self.directory(cx);
        let filter = self.filter(cx);
        let reply = self.cache.get(&directory).cloned();
        let rows = self.rows(cx);
        let focused = self.input.read(cx).focus_handle(cx).is_focused(window);
        let accent = theme.accent;

        let field = div()
            .id(SharedString::from(format!("folder-field-{}", self.label)))
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h_8()
            .px_2p5()
            .rounded(theme.radius.md)
            .border_1()
            .border_color(if focused { theme.primary } else { theme.border })
            .font_family(theme.mono_font_family)
            .text_token(TextSize::Xs)
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.arrow(true, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.arrow(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &IndentInline, window, cx| {
                if this.tab(window, cx) {
                    cx.stop_propagation();
                }
            }))
            .child(bare_input(&self.input, TextSize::Xs))
            .test_support();

        let browse = can_pick.then(|| {
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
                .hover(move |style| style.bg(accent))
                .tooltip(|window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(
                        "Choose with the system's folder dialog",
                    )
                    .build(window, cx)
                })
                .on_click(cx.listener(|this, _, window, cx| this.browse(window, cx)))
                .child(Icon::new(IconName::FolderSearch).size(px(14.)))
                .child("Browse…")
                .test_support()
        });

        let list = self.show_list().then(|| {
            let shortcuts: Vec<(String, String, bool)> = self
                .last_listing
                .as_ref()
                .map(|listing| {
                    std::iter::once(("Home".to_string(), listing.home.clone(), true))
                        .chain(
                            listing
                                .roots
                                .iter()
                                .map(|root| (root.clone(), root.clone(), false)),
                        )
                        .collect()
                })
                .unwrap_or_default();
            let show_shortcuts =
                shortcuts.len() > 1 || shortcuts.first().is_some_and(|(_, _, home)| !home);
            let foreground = theme.foreground;
            let chips = shortcuts
                .into_iter()
                .enumerate()
                .map(|(index, (label, path, home))| {
                    div()
                        .id(("folder-shortcut", index))
                        .flex()
                        .items_center()
                        .gap_1()
                        .h_6()
                        .px_2()
                        .rounded(theme.radius.md)
                        .bg(theme.accent)
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .cursor_pointer()
                        .hover(move |style| style.text_color(foreground))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.set_value(path.clone(), window, cx)
                        }))
                        .when(home, |this| this.child(Icon::new(IconName::House).size(px(12.))))
                        .child(label)
                        .test_support()
                });
            let message = |text: String, color| {
                div()
                    .px_2()
                    .py_4()
                    .text_center()
                    .text_token(TextSize::Xs)
                    .text_color(color)
                    .child(text)
                    .into_any_element()
            };
            let body: Vec<gpui::AnyElement> = match &reply {
                Some(OrError::Error { error }) => vec![message(error.clone(), theme.destructive)],
                _ if directory.is_empty() => vec![message(
                    "Start the path with a drive, / or ~.".into(),
                    theme.muted_foreground,
                )],
                None => vec![message(
                    format!("Looking in {directory}…"),
                    theme.muted_foreground,
                )],
                Some(OrError::Ok(_)) if rows.is_empty() => vec![message(
                    if filter.is_empty() {
                        "No folders in here.".into()
                    } else {
                        format!("No folder here starts with \"{filter}\".")
                    },
                    theme.muted_foreground,
                )],
                Some(OrError::Ok(_)) => rows
                    .iter()
                    .enumerate()
                    .map(|(index, row)| {
                        let highlighted = self.highlight == Some(index);
                        let picked = row.clone();
                        div()
                            .id(("folder-row", index))
                            .flex()
                            .items_center()
                            .gap_2()
                            .w_full()
                            .px_2()
                            .py_1()
                            .rounded(theme.radius.md)
                            .text_token(TextSize::Sm)
                            .cursor_pointer()
                            .map(|this| {
                                if highlighted {
                                    this.bg(theme.accent).text_color(theme.foreground)
                                } else {
                                    this.hover(move |style| style.bg(accent.opacity(0.6)))
                                }
                            })
                            .on_mouse_move(cx.listener(move |this, _, _, cx| {
                                if this.highlight != Some(index) {
                                    this.highlight = Some(index);
                                    cx.notify();
                                }
                            }))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.activate(picked.clone(), window, cx)
                            }))
                            .map(|this| match row {
                                Row::Up(_) => this
                                    .child(
                                        Icon::new(IconName::CornerLeftUp)
                                            .size(px(14.))
                                            .text_color(theme.muted_foreground),
                                    )
                                    .child(div().text_color(theme.muted_foreground).child("..")),
                                Row::Entry(entry) => this
                                    .child(
                                        Icon::new(if entry.is_project {
                                            IconName::FolderCheck
                                        } else {
                                            IconName::Folder
                                        })
                                        .size(px(14.))
                                        .text_color(if entry.is_project {
                                            theme.primary
                                        } else {
                                            theme.muted_foreground
                                        }),
                                    )
                                    .child(div().min_w_0().truncate().child(entry.name.clone()))
                                    .when(entry.is_project, |this| {
                                        this.child(
                                            div()
                                                .ml_auto()
                                                .flex_shrink_0()
                                                .rounded(theme.radius.sm)
                                                .bg(theme.primary.opacity(0.15))
                                                .px_1p5()
                                                .text_token(TextSize::X2s)
                                                .text_color(theme.primary)
                                                .child("Linker-Edit project"),
                                        )
                                    }),
                            })
                            .test_support()
                            .into_any_element()
                    })
                    .collect(),
            };
            div()
                .flex()
                .flex_col()
                .gap_2()
                .when(show_shortcuts, |this| {
                    this.child(div().flex().flex_wrap().gap_1().children(chips))
                })
                .child(
                    div()
                        .id(SharedString::from(format!("folders-{}", self.label)))
                        .mx(px(-4.))
                        .h(if self.list_on_focus { px(160.) } else { px(224.) })
                        .overflow_y_scroll()
                        .children(body)
                        .test_support(),
                )
                .child(
                    div()
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .child("↑↓ choose a folder · Enter or Tab opens it · Enter with none chosen picks this path"),
                )
        });

        div()
            .flex()
            .flex_col()
            .gap_2()
            .child(div().flex().gap_1p5().child(field).children(browse))
            .children(list)
    }
}
