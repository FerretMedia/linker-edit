//! LibraryPage.svelte: the library page, which replaces the app while open, over the server's
//! library_* messages. Two tabs (Universal, This project); a library's folders (rescan, edit,
//! remove, add) and its files (search, kind, starred; play or a thumbnail; star; tags and note).
//! It asks for its library while open, after a reconnect, a switch of tab or project and a new
//! search (typing waits 150ms); the server sends every change after, until `library_close`.
//!
//! Pictures come from the server's /library/… over HTTP. Sound plays in the player that the
//! preview's port brings: the page emits [`LibraryEvent::Play`] and shows Pause until it hears
//! [`LibraryPage::stopped`], as the Svelte page's `<audio>` does with its pause and ended events.

use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, InteractiveElement,
    IntoElement, ObjectFit, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, StyledImage, Subscription, Task, Window, div, img, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
use gpui_kit::component::select::{Select, SelectEvent};
use gpui_kit::component::tooltip::Tooltip;
use linker_edit_protocol::{
    ClientMessage, LIBRARY_KINDS, LibraryClientMessage, LibraryFile, LibraryFolder,
    LibraryFolderChanges, LibraryFolderStatus, LibraryKind, LibraryQuery, LibraryScope,
    LibraryView,
};

use crate::icons::IconName;
use crate::library::{
    LibraryTab, file_url, format_duration, kind_label, parse_tags, playable, scope_of, showable,
};
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::choice_select::{Choice, ChoiceState, choice_state};
use crate::views::library_folder_dialog::{FolderDialogEvent, LibraryFolderDialog};
use crate::views::settings_parts::{
    PageFrame, confirm_then, divided, ghost_button, group, medium_date_time, nav_tab,
    outline_button, page_title, primary_button, progress_bar,
};

const TYPING: Duration = Duration::from_millis(150);

pub enum LibraryEvent {
    Close,
    /// Play this file's address; one plays at a time.
    Play(String),
    /// Stop the one playing.
    Stop,
}

fn kind_icon(kind: LibraryKind) -> IconName {
    match kind {
        LibraryKind::Music => IconName::Music,
        LibraryKind::Sfx => IconName::AudioWaveform,
        LibraryKind::Graphics => IconName::Image,
        LibraryKind::Luts => IconName::Palette,
        LibraryKind::Footage => IconName::Film,
        LibraryKind::Other => IconName::Folder,
    }
}

fn key_of(file: &LibraryFile) -> String {
    format!("{}/{}", file.folder_id, file.rel)
}

/// The library and query asked for, and whether the link was up, to ask again on a change.
type Asked = (bool, Option<LibraryScope>, LibraryQuery);

pub struct LibraryPage {
    state: Entity<AppState>,
    tab: LibraryTab,
    search: Entity<InputState>,
    /// The search as the server was asked it, a moment after typing stops.
    typed: String,
    typing: Option<Task<()>>,
    kind: Entity<ChoiceState>,
    starred: bool,
    asked: Option<Asked>,
    dialog: Option<Entity<LibraryFolderDialog>>,
    playing: Option<String>,
    /// The file whose tags and note are open.
    open: Option<String>,
    tags: Entity<InputState>,
    note: Entity<TextareaState>,
    /// Takes the keyboard back when the folder dialog closes, so Escape still leaves the page.
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LibraryEvent> for LibraryPage {}

impl LibraryPage {
    pub fn new(
        state: Entity<AppState>,
        tab: LibraryTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder("Search names, tags and notes"));
        let kinds = std::iter::once(Choice::new("", "All kinds"))
            .chain(
                LIBRARY_KINDS
                    .iter()
                    .map(|&kind| Choice::new(kind_value(kind), kind_label(kind))),
            )
            .collect();
        let kind = choice_state(kinds, "", window, cx);
        let tags = cx.new(|cx| InputState::new(window, cx).placeholder("upbeat, intro"));
        let note = cx.new(|cx| {
            TextareaState::new(window, cx)
                .auto_grow(2, 8)
                .placeholder("Good under talking heads.")
        });
        let subscriptions = vec![
            cx.observe(&state, |this, _, cx| this.ask(cx)),
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.typed_later(cx);
                }
            }),
            cx.subscribe(&kind, |this, _, _: &SelectEvent<Vec<Choice>>, cx| {
                this.ask(cx)
            }),
            cx.subscribe_in(&tags, window, |this, _, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && let Some(file) = this.open_file(cx)
                {
                    this.save_file(&file, window, cx);
                }
            }),
            // Leaving the page stops the server sending its library, and the sound.
            cx.on_release(|this, cx| {
                this.state
                    .read(cx)
                    .send(ClientMessage::Library(LibraryClientMessage::LibraryClose));
            }),
        ];
        let mut page = LibraryPage {
            state,
            tab,
            search,
            typed: String::new(),
            typing: None,
            kind,
            starred: false,
            asked: None,
            dialog: None,
            playing: None,
            open: None,
            tags,
            note,
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        page.ask(cx);
        page
    }

    pub fn has_dialog(&self) -> bool {
        self.dialog.is_some()
    }

    pub fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    pub fn tab(&self) -> LibraryTab {
        self.tab
    }

    pub fn show(&mut self, tab: LibraryTab, cx: &mut Context<Self>) {
        self.tab = tab;
        self.ask(cx);
    }

    /// The player says the sound stopped (paused, ended or failed).
    pub fn stopped(&mut self, cx: &mut Context<Self>) {
        self.playing = None;
        cx.notify();
    }

    pub fn playing(&self) -> Option<&str> {
        self.playing.as_deref()
    }

    fn scope(&self, cx: &App) -> Option<LibraryScope> {
        scope_of(self.tab, self.state.read(cx).project_id())
    }

    fn query(&self, cx: &App) -> LibraryQuery {
        let kind = self
            .kind
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or_default();
        let typed = self.typed.trim();
        LibraryQuery {
            query: (!typed.is_empty()).then(|| typed.to_string()),
            kind: serde_json::from_value(serde_json::Value::String(kind)).ok(),
            starred: self.starred.then_some(true),
            ..Default::default()
        }
    }

    /// The view the server sent, when it is of this page's library.
    fn current(&self, cx: &App) -> Option<LibraryView> {
        let scope = self.scope(cx)?;
        self.state
            .read(cx)
            .library_view
            .clone()
            .filter(|view| view.scope == scope)
    }

    fn ask(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let now: Asked = (state.connected, self.scope(cx), self.query(cx));
        if self.asked.as_ref() != Some(&now) {
            if let (true, Some(scope)) = (now.0, now.1.clone()) {
                state.send(ClientMessage::Library(LibraryClientMessage::LibraryOpen {
                    scope,
                    query: now.2.clone(),
                }));
            }
            self.asked = Some(now);
        }
        cx.notify();
    }

    fn typed_later(&mut self, cx: &mut Context<Self>) {
        let value = self.search.read(cx).value().to_string();
        self.typing = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(TYPING).await;
            let _ = this.update(cx, |this, cx| {
                this.typed = value;
                this.ask(cx);
            });
        }));
    }

    fn open_dialog(
        &mut self,
        folder: Option<LibraryFolder>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self
            .state
            .read(cx)
            .project()
            .map(|project| project.name.clone());
        let where_ = match self.tab {
            LibraryTab::Universal => "the universal library, for every project".to_string(),
            LibraryTab::Project => format!(
                "the library of \"{}\"",
                project.as_deref().unwrap_or("this project")
            ),
        };
        let editing = folder.as_ref().map(|folder| folder.id.clone());
        let dialog =
            cx.new(|cx| LibraryFolderDialog::new(self.state.clone(), folder, where_, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &dialog,
            window,
            move |this, _, event: &FolderDialogEvent, window, cx| {
                match event {
                    FolderDialogEvent::Close => this.close_dialog(window, cx),
                    FolderDialogEvent::Save(fields) => {
                        let Some(scope) = this.scope(cx) else { return };
                        let message = match &editing {
                            Some(folder_id) => LibraryClientMessage::LibraryUpdateFolder {
                                scope,
                                folder_id: folder_id.clone(),
                                changes: LibraryFolderChanges {
                                    kind: Some(fields.kind),
                                    label: Some(fields.label.clone()),
                                    path: Some(fields.path.clone()),
                                    note: Some(fields.note.clone()),
                                },
                            },
                            None => LibraryClientMessage::LibraryAddFolder {
                                scope,
                                folder: fields.clone(),
                            },
                        };
                        this.state.read(cx).send(ClientMessage::Library(message));
                    }
                }
                cx.notify();
            },
        ));
        self.dialog = Some(dialog);
        cx.notify();
    }

    fn toggle_play(&mut self, file: &LibraryFile, cx: &mut Context<Self>) {
        let key = key_of(file);
        if self.playing.as_ref() == Some(&key) {
            self.playing = None;
            cx.emit(LibraryEvent::Stop);
        } else if let Some(url) = self
            .state
            .read(cx)
            .server_url(&file_url(&file.folder_id, &file.rel))
        {
            self.playing = Some(key);
            cx.emit(LibraryEvent::Play(url));
        }
        cx.notify();
    }

    fn open_file(&self, cx: &App) -> Option<LibraryFile> {
        let open = self.open.as_ref()?;
        self.current(cx)?
            .files
            .into_iter()
            .find(|file| &key_of(file) == open)
    }

    fn edit(&mut self, file: &LibraryFile, window: &mut Window, cx: &mut Context<Self>) {
        let key = key_of(file);
        if self.open.as_ref() == Some(&key) {
            self.open = None;
        } else {
            self.open = Some(key);
            let (tags, note) = (file.tags.join(", "), file.note.clone());
            self.tags
                .update(cx, |input, cx| input.set_value(tags, window, cx));
            self.note
                .update(cx, |input, cx| input.set_value(note, window, cx));
        }
        cx.notify();
    }

    fn save_file(&mut self, file: &LibraryFile, _: &mut Window, cx: &mut Context<Self>) {
        let tags = parse_tags(&self.tags.read(cx).value());
        let note = self.note.read(cx).value().to_string();
        self.state.read(cx).send(ClientMessage::Library(
            LibraryClientMessage::LibraryUpdateFile {
                folder_id: file.folder_id.clone(),
                rel: file.rel.clone(),
                star: None,
                tags: Some(tags),
                note: Some(note),
            },
        ));
        self.open = None;
        cx.notify();
    }

    fn folder_row(
        &self,
        folder: &LibraryFolderStatus,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = folder.folder.id.clone();
        let files = if let Some(scanning) = folder.scanning {
            format!(
                "Reading {} of {}",
                scanning.done,
                if scanning.total > 0 {
                    scanning.total.to_string()
                } else {
                    "…".into()
                }
            )
        } else {
            format!(
                "{} {}{}",
                folder.files,
                if folder.files == 1 { "file" } else { "files" },
                folder
                    .scanned_at
                    .as_deref()
                    .map(|at| format!(", read {}", medium_date_time(at)))
                    .unwrap_or_default()
            )
        };
        let (accent, foreground, destructive) = (theme.accent, theme.foreground, theme.destructive);
        let icon_button =
            |id: SharedString, icon: IconName, tip: &'static str, hover: gpui::Hsla| {
                div()
                    .id(id)
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .size_7()
                    .rounded(theme.radius.md)
                    .text_color(theme.muted_foreground)
                    .cursor_pointer()
                    .hover(move |style| style.bg(accent).text_color(hover))
                    .tooltip(move |window, cx| Tooltip::new(tip).build(window, cx))
                    .child(Icon::new(icon).size(px(14.)))
            };
        let scanning = folder.scanning.is_some();
        let (rescan_id, edit_folder, remove) = (id.clone(), folder.folder.clone(), folder.clone());
        div()
            .id(SharedString::from(format!("folder-{id}")))
            .flex()
            .items_start()
            .gap_4()
            .p_4()
            .child(
                Icon::new(kind_icon(folder.folder.kind))
                    .size(px(16.))
                    .mt_0p5()
                    .flex_shrink_0()
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_token(TextSize::Sm).child(folder.folder.label.clone()))
                    .child(
                        div()
                            .mt_0p5()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child(format!("{} · {files}", kind_label(folder.folder.kind))),
                    )
                    .child(
                        div()
                            .mt_1()
                            .truncate()
                            .font_family(theme.mono_font_family)
                            .text_token(TextSize::X2s)
                            .text_color(theme.muted_foreground)
                            .child(folder.folder.path.clone()),
                    )
                    .when(!folder.folder.note.is_empty(), |this| {
                        this.child(div().mt_1().text_token(TextSize::Xs).child(folder.folder.note.clone()))
                    })
                    .when_some(folder.problem.clone(), |this, problem| {
                        this.child(
                            div()
                                .mt_1()
                                .text_token(TextSize::Xs)
                                .text_color(theme.destructive)
                                .child(problem),
                        )
                    })
                    .when_some(folder.scanning.filter(|s| s.total > 0), |this, scanning| {
                        this.child(div().mt_2().child(progress_bar(
                            Some(f64::from(scanning.done) / f64::from(scanning.total) * 100.),
                            theme,
                        )))
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1()
                    .child(
                        icon_button(SharedString::from(format!("rescan-{id}")), IconName::RefreshCw, "Rescan: reads new and changed files", foreground)
                            .when(!scanning, |this| {
                                this.on_click(cx.listener(move |this, _, _, cx| {
                                    this.state.read(cx).send(ClientMessage::Library(
                                        LibraryClientMessage::LibraryRescan { folder_id: rescan_id.clone() },
                                    ))
                                }))
                            })
                            .test_support(),
                    )
                    .child(
                        icon_button(SharedString::from(format!("edit-{id}")), IconName::Pencil, "Edit", foreground)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_dialog(Some(edit_folder.clone()), window, cx)
                            }))
                            .test_support(),
                    )
                    .child(
                        icon_button(SharedString::from(format!("remove-{id}")), IconName::Trash2, "Remove from the library. The folder stays.", destructive)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let Some(scope) = this.scope(cx) else { return };
                                let (state, folder_id) = (this.state.clone(), remove.folder.id.clone());
                                confirm_then(
                                    &format!(
                                        "Remove \"{}\" from the library? The folder and its files stay where they are.",
                                        remove.folder.label
                                    ),
                                    window,
                                    cx,
                                    move |cx| {
                                        state.read(cx).send(ClientMessage::Library(
                                            LibraryClientMessage::LibraryRemoveFolder { scope, folder_id },
                                        ))
                                    },
                                );
                            }))
                            .test_support(),
                    ),
            )
            .test_support()
            .into_any_element()
    }

    fn file_row(
        &self,
        file: &LibraryFile,
        view: &LibraryView,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let key = key_of(file);
        let (accent, foreground) = (theme.accent, theme.foreground);
        let playing = self.playing.as_ref() == Some(&key);
        let folder_label = view
            .folders
            .iter()
            .find(|folder| folder.folder.id == file.folder_id)
            .map(|folder| folder.folder.label.clone());
        let dir = file
            .rel
            .rsplit_once('/')
            .map(|(dir, _)| dir.to_string())
            .unwrap_or_default();
        let line: Vec<String> = [
            folder_label.filter(|_| view.folders.len() > 1),
            (!dir.is_empty()).then_some(dir),
            file.duration.map(format_duration),
            file.loudness.map(|lufs| format!("{} LUFS", lufs.round())),
            match file.channels {
                Some(1) => Some("mono".into()),
                Some(2) => Some("stereo".into()),
                Some(0) | None => None,
                Some(channels) => Some(format!("{channels} ch")),
            },
            (!file.used_in.is_empty()).then(|| format!("used {}×", file.used_in.len())),
        ]
        .into_iter()
        .flatten()
        .collect();
        let leading = if showable(&file.name) {
            match self
                .state
                .read(cx)
                .server_url(&file_url(&file.folder_id, &file.rel))
            {
                Some(url) => img(url)
                    .size_7()
                    .flex_shrink_0()
                    .rounded(theme.radius.sm)
                    .object_fit(ObjectFit::Cover)
                    .into_any_element(),
                None => div().size_7().flex_shrink_0().into_any_element(),
            }
        } else {
            let can_play = playable(&file.name);
            let play_file = file.clone();
            div()
                .id(SharedString::from(format!("play-{key}")))
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size_7()
                .rounded(theme.radius.md)
                .text_color(theme.muted_foreground)
                .when(!can_play, |this| this.opacity(0.3))
                .when(can_play, |this| {
                    this.cursor_pointer()
                        .hover(move |style| style.bg(accent).text_color(foreground))
                        .on_click(
                            cx.listener(move |this, _, _, cx| this.toggle_play(&play_file, cx)),
                        )
                })
                .child(
                    Icon::new(if playing {
                        IconName::Pause
                    } else {
                        IconName::Play
                    })
                    .size(px(14.)),
                )
                .test_support()
                .into_any_element()
        };
        let (edit_file, star_file) = (file.clone(), file.clone());
        let open = self.open.as_ref() == Some(&key);
        let row = div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .py_2()
            .child(leading)
            .child(
                div()
                    .id(SharedString::from(format!("file-{key}")))
                    .flex_1()
                    .min_w_0()
                    .cursor_pointer()
                    .tooltip(|window, cx| Tooltip::new("Tags and note").build(window, cx))
                    .on_click(
                        cx.listener(move |this, _, window, cx| this.edit(&edit_file, window, cx)),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_token(TextSize::Sm)
                            .child(file.name.clone()),
                    )
                    // An empty line takes no room, as an empty <p> doesn't.
                    .when(!line.is_empty(), |this| {
                        this.child(
                            div()
                                .truncate()
                                .text_token(TextSize::X2s)
                                .text_color(theme.muted_foreground)
                                .child(line.join(" · ")),
                        )
                    })
                    .test_support(),
            )
            .children(file.tags.iter().take(3).map(|tag| {
                div()
                    .flex_shrink_0()
                    .rounded(theme.radius.sm)
                    .bg(theme.accent)
                    .px_1p5()
                    .text_token(TextSize::X2s)
                    .text_color(theme.muted_foreground)
                    .child(tag.clone())
            }))
            .child(
                div()
                    .id(SharedString::from(format!("star-{key}")))
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .size_7()
                    .rounded(theme.radius.md)
                    .text_color(if file.star {
                        theme.warning
                    } else {
                        theme.muted_foreground
                    })
                    .cursor_pointer()
                    .hover(move |style| style.bg(accent))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.state.read(cx).send(ClientMessage::Library(
                            LibraryClientMessage::LibraryUpdateFile {
                                folder_id: star_file.folder_id.clone(),
                                rel: star_file.rel.clone(),
                                star: Some(!star_file.star),
                                tags: None,
                                note: None,
                            },
                        ))
                    }))
                    .child(
                        Icon::new(if file.star {
                            IconName::StarFilled
                        } else {
                            IconName::Star
                        })
                        .size(px(14.)),
                    )
                    .test_support(),
            );
        let details = open.then(|| {
            let save_file = file.clone();
            let used = (!file.used_in.is_empty()).then(|| {
                format!(
                    "Used in {}",
                    file.used_in
                        .iter()
                        .map(|used| match &used.timeline {
                            Some(timeline) => format!("{} · {timeline}", used.project),
                            None => used.project.clone(),
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            });
            div()
                .id(SharedString::from(format!("details-{key}")))
                .flex()
                .flex_col()
                .gap_2()
                .border_t_1()
                .border_color(theme.border)
                .bg(theme.muted.opacity(0.4))
                .px_3()
                .py_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child("Tags, with commas")
                        .child(
                            div()
                                .text_color(theme.foreground)
                                .child(Input::new(&self.tags)),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child("Note")
                        .child(
                            div()
                                .rounded(theme.radius.md)
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.card)
                                .px_2p5()
                                .py_1p5()
                                .text_color(theme.foreground)
                                .child(Textarea::new(&self.note).appearance(false)),
                        ),
                )
                .when_some(used, |this, used| {
                    this.child(
                        div()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child(used),
                    )
                })
                .child(
                    div()
                        .truncate()
                        .font_family(theme.mono_font_family)
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .child(file.path.clone()),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            ghost_button("cancel-file", theme)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.open = None;
                                    cx.notify();
                                }))
                                .child("Cancel")
                                .test_support(),
                        )
                        .child(
                            primary_button("save-file", theme)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.save_file(&save_file, window, cx)
                                }))
                                .child("Save")
                                .test_support(),
                        ),
                )
                .test_support()
        });
        div()
            .flex()
            .flex_col()
            .child(row)
            .children(details)
            .into_any_element()
    }
}

fn kind_value(kind: LibraryKind) -> String {
    serde_json::to_value(kind)
        .ok()
        .and_then(|value| value.as_str().map(String::from))
        .unwrap_or_default()
}

impl Render for LibraryPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let state = self.state.read(cx);
        let connected = state.connected;
        let project = state.project().map(|project| project.name.clone());
        let has_project = project.is_some();
        let scope = self.scope(cx);
        let current = self.current(cx);

        let tabs = [
            (
                LibraryTab::Universal,
                "library-universal",
                IconName::Globe,
                "Universal",
            ),
            (
                LibraryTab::Project,
                "library-project",
                IconName::FolderOpen,
                "This project",
            ),
        ]
        .into_iter()
        .map(|(tab, id, icon, label)| {
            let disabled = tab == LibraryTab::Project && !has_project;
            let tip: SharedString = match tab {
                LibraryTab::Universal => "Every project".into(),
                LibraryTab::Project => project
                    .clone()
                    .unwrap_or("Add a project first".into())
                    .into(),
            };
            nav_tab(id, icon, label, tab == self.tab, &theme)
                .when(disabled, |this| this.opacity(0.5).cursor_default())
                .when(!disabled, |this| {
                    this.on_click(cx.listener(move |this, _, _, cx| this.show(tab, cx)))
                })
                .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                .test_support()
                .into_any_element()
        })
        .collect();

        let title = match self.tab {
            LibraryTab::Universal => "Universal library".to_string(),
            LibraryTab::Project => format!(
                "Library of {}",
                project.as_deref().unwrap_or("this project")
            ),
        };
        let mut header = vec![page_title(title).into_any_element()];
        if scope.is_some() {
            header.push(
                crate::window::no_drag()
                    .ml_auto()
                    .child(
                        outline_button("add-folder", &theme)
                            .on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.open_dialog(None, window, cx)
                                }),
                            )
                            .child(Icon::new(IconName::Plus).size(px(14.)))
                            .child("Add folder")
                            .test_support(),
                    )
                    .into_any_element(),
            );
        }

        let muted = |text: &str| {
            div()
                .text_token(TextSize::Sm)
                .text_color(theme.muted_foreground)
                .child(text.to_string())
                .into_any_element()
        };
        let content: Vec<AnyElement> = match (&scope, &current) {
            (None, _) => vec![muted(
                "Add a project first. Its own library is for files only that project uses.",
            )],
            (Some(_), None) => vec![muted(if connected {
                "Loading…"
            } else {
                "Connecting to the Linker-Edit server…"
            })],
            (Some(_), Some(view)) => {
                let folders: Vec<AnyElement> = if view.folders.is_empty() {
                    vec![div()
                        .flex()
                        .flex_col()
                        .items_start()
                        .gap_3()
                        .p_4()
                        .child(muted("No folders yet. Add the folders where you keep music, sound effects, logos or LUTs."))
                        .child(
                            outline_button("add-first-folder", &theme)
                                .on_click(cx.listener(|this, _, window, cx| this.open_dialog(None, window, cx)))
                                .child(Icon::new(IconName::Plus).size(px(14.)))
                                .child("Add folder")
                                .test_support(),
                        )
                        .into_any_element()]
                } else {
                    view.folders
                        .iter()
                        .map(|folder| self.folder_row(folder, &theme, cx))
                        .collect()
                };
                let mut content = vec![group(
                    "Folders",
                    Some(match self.tab {
                        LibraryTab::Universal => "Every project can use these. Linker-Edit never copies, moves or deletes your files.".into(),
                        LibraryTab::Project => "Only this project uses these, before the universal library. Kept on this computer only.".into(),
                    }),
                    folders,
                    &theme,
                )
                .into_any_element()];
                if !view.folders.is_empty() {
                    let filtered = !self.typed.is_empty()
                        || self
                            .kind
                            .read(cx)
                            .selected_value()
                            .is_some_and(|kind| !kind.is_empty())
                        || self.starred;
                    let files: Vec<AnyElement> = if view.files.is_empty() {
                        vec![
                            div()
                                .p_4()
                                .text_center()
                                .text_token(TextSize::Sm)
                                .text_color(theme.muted_foreground)
                                .child(
                                    if view.folders.iter().any(|folder| folder.scanning.is_some()) {
                                        "Reading the folders…"
                                    } else if filtered {
                                        "Nothing matches."
                                    } else {
                                        "No files in these folders."
                                    },
                                )
                                .into_any_element(),
                        ]
                    } else {
                        view.files
                            .iter()
                            .map(|file| self.file_row(file, view, &theme, cx))
                            .collect()
                    };
                    let starred = self.starred;
                    let accent = theme.accent;
                    content.push(
                        div()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .flex_wrap()
                                    .items_center()
                                    .gap_2()
                                    .child(
                                        div().flex().flex_1().min_w(px(192.)).child(
                                            Input::new(&self.search).prefix(
                                                Icon::new(IconName::Search)
                                                    .size(px(14.))
                                                    .text_color(theme.muted_foreground),
                                            ),
                                        ),
                                    )
                                    .child(div().w(px(150.)).child(Select::new(&self.kind)))
                                    .child(
                                        outline_button("starred", &theme)
                                            .when(starred, |this| {
                                                this.bg(accent).text_color(theme.foreground)
                                            })
                                            .when(!starred, |this| {
                                                this.text_color(theme.muted_foreground)
                                            })
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.starred = !this.starred;
                                                this.ask(cx);
                                            }))
                                            .child(
                                                Icon::new(if starred {
                                                    IconName::StarFilled
                                                } else {
                                                    IconName::Star
                                                })
                                                .size(px(14.))
                                                .when(starred, |icon| {
                                                    icon.text_color(theme.warning)
                                                }),
                                            )
                                            .child("Starred")
                                            .test_support(),
                                    ),
                            )
                            .child(
                                div()
                                    .id("library-files")
                                    .flex()
                                    .flex_col()
                                    .rounded(theme.radius.xl)
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(theme.card)
                                    .children(divided(files, &theme))
                                    .test_support(),
                            )
                            .when(view.total as usize > view.files.len(), |this| {
                                this.child(
                                    div()
                                        .px_1()
                                        .text_token(TextSize::Xs)
                                        .text_color(theme.muted_foreground)
                                        .child(format!(
                                            "Showing {} of {}. Search to narrow it down.",
                                            view.files.len(),
                                            view.total
                                        )),
                                )
                            })
                            .into_any_element(),
                    );
                }
                content
            }
        };

        PageFrame {
            id: "library-page",
            on_back: Box::new(cx.listener(|_, _, _, cx| cx.emit(LibraryEvent::Close))),
            tabs,
            footer: div()
                .child("Files reused across your edits. Agents look here first for music, sound effects and graphics.")
                .into_any_element(),
            header,
            content,
            max_width: px(768.),
        }
        .render(window, cx)
        .relative()
        .track_focus(&self.focus)
        .children(self.dialog.clone())
        .test_support()
    }
}
