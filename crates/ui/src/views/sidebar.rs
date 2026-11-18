//! Sidebar.svelte: a copy of T3 Code's sidebar (apps/web/src/components/Sidebar.tsx and
//! sidebar/*; MIT, see NOTICE) in its project-folder layout: pinned threads, a folder per project,
//! then the Snoozed and Settled shelves. Linker-Edit's own parts (the computer menu, Resolve,
//! transcription, updates, the Linker account) stay around it. 256px wide, like its `w-64`.
//!
//! The rows are in sidebar_rows.rs and the foot in sidebar_footer.rs; the logic (sections,
//! sorting, unread, snoozing, search) is crate::sidebar, the port of src/lib/sidebar.ts.
//!
//! What is only the window's business lives here, as in the Svelte component: which folders and
//! shelves are open, the search, the row being renamed, the open menu, the jump hints. Those the
//! browser keeps in localStorage (closed folders, open shelves) last as long as the window here.

use std::collections::HashSet;
use std::time::Duration;

use chrono::Local;
use gpui::{
    AnyElement, AppContext, ClipboardItem, Context, Entity, EventEmitter, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ModifiersChangedEvent, ParentElement, Pixels,
    Point, PromptLevel, Render, StatefulInteractiveElement, Styled, Subscription, Task, Window,
    div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::base::input::{Escape, MoveDown, MoveUp};
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use linker_edit_protocol::release_name::release_name;
use linker_edit_protocol::{
    ClientMessage, DesktopUpdateStatus, ProjectSidebarChanges, ProjectSummary, ResolveStatus,
    SidebarClientMessage, ThreadSummary,
};

use crate::components::LinkerLogo;
use crate::icons::IconName;
use crate::sidebar::{
    FOLDER_PREVIEW_COUNT, SETTLED_INITIAL_COUNT, SidebarLists, SidebarThread, is_unread, is_woke,
    next_thread_after, next_wake, now_ms, partition_sidebar, search_threads, snooze_presets, time,
};
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::computer_picker::{ComputerPicker, ComputerPickerEvent};
use crate::views::context_menu::{ContextMenu, MenuEvent, MenuItem};
use crate::views::dialog::bare_input;
use crate::window::{no_drag, title_bar};

pub const SIDEBAR_WIDTH: Pixels = px(256.);
/// --sidebar-content-inset
pub(super) const INSET: Pixels = px(8.);

/// What the sidebar asks of the rest of the window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SidebarEvent {
    /// A new thread in this project: the workspace sends it and puts the cursor in the composer.
    NewThread(String),
    AddProject,
    ProjectSettings(String),
    Settings,
    RemoteAccess,
}

/// What a menu row does.
#[derive(Clone, Debug, PartialEq)]
pub enum SidebarAction {
    Pin(String, bool),
    Settle(String, bool),
    Snooze(String, Option<String>),
    Rename(String),
    MarkUnread(String),
    Copy(String),
    ProjectSettings(String),
    Archive(String, bool),
    Delete(String),
    NewThread(String),
    UpdateProject(String, ProjectSidebarChanges),
    OpenInResolve(String),
    OpenUrl(String),
    SignOut,
}

/// Which row variant: "card" for pinned threads, "folder" in a project folder, "slim" on a shelf.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Variant {
    Card,
    Folder,
    Slim,
}

/// The action a row offers on hover: settle, un-settle, or wake.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowAction {
    Settle,
    Unsettle,
    Unsnooze,
}

pub struct Sidebar {
    pub(super) state: Entity<AppState>,
    pub(super) computer_picker: Entity<ComputerPicker>,
    /// A minute clock, like T3 Code's, and an exact tick when a snoozed thread wakes.
    pub(super) now: i64,
    _clock: Task<()>,
    wake: Option<(i64, Task<()>)>,
    pub(super) collapsed: HashSet<String>,
    pub(super) settled_expanded: bool,
    pub(super) snoozed_expanded: bool,
    pub(super) expanded_folders: HashSet<String>,
    pub(super) settled_shown: usize,
    pub(super) search: Entity<InputState>,
    pub(super) active_result: usize,
    /// The thread being renamed, and its title field.
    pub(super) renaming: Option<(String, Entity<InputState>)>,
    menu: Option<(Entity<ContextMenu<SidebarAction>>, Subscription)>,
    pub(super) jump_hints: bool,
    hint_timer: Option<Task<()>>,
    /// The row or folder under the pointer, which shows its buttons.
    pub(super) hovered: Option<String>,
    /// The user just asked to check for updates, so the card also says when there's nothing new.
    pub(super) asked: bool,
    asked_timer: Option<Task<()>>,
    /// The thread last marked read, with what made it unread, so it is sent once.
    marked: Option<(String, Option<String>, Option<String>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SidebarEvent> for Sidebar {}

impl Sidebar {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let computer_picker = cx.new(|cx| ComputerPicker::new(state.clone(), window, cx));
        Self::with_picker(state, computer_picker, window, cx)
    }

    pub fn with_picker(
        state: Entity<AppState>,
        computer_picker: Entity<ComputerPicker>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search"));
        let subscriptions = vec![
            cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx)),
            cx.subscribe_in(
                &search,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::Change => this.active_result = 0,
                        InputEvent::PressEnter { .. } => this.open_highlighted(window, cx),
                        _ => {}
                    }
                    cx.notify();
                },
            ),
            cx.subscribe(
                &computer_picker,
                |_, _, event: &ComputerPickerEvent, cx| match event {
                    ComputerPickerEvent::RemoteAccess => cx.emit(SidebarEvent::RemoteAccess),
                },
            ),
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.hide_hints(cx);
                    this.close_menu(cx);
                }
                this.mark_read(window, cx);
            }),
        ];
        let clock = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_secs(60))
                    .await;
                if this
                    .update(cx, |this, cx| {
                        this.now = now_ms();
                        cx.notify()
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let mut sidebar = Sidebar {
            state,
            computer_picker,
            now: now_ms(),
            _clock: clock,
            wake: None,
            collapsed: HashSet::new(),
            settled_expanded: false,
            snoozed_expanded: false,
            expanded_folders: HashSet::new(),
            settled_shown: SETTLED_INITIAL_COUNT,
            search,
            active_result: 0,
            renaming: None,
            menu: None,
            jump_hints: false,
            hint_timer: None,
            hovered: None,
            asked: false,
            asked_timer: None,
            marked: None,
            _subscriptions: subscriptions,
        };
        sidebar.sync(window, cx);
        sidebar
    }

    /// Follows the server: the clock, the wake timer, "asked", and marking the open thread read.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.now = now_ms();
        let state = self.state.read(cx);
        let projects = state
            .app
            .as_ref()
            .map(|app| app.meta.projects.as_slice())
            .unwrap_or_default();
        let wake = next_wake(projects, self.now);
        if wake != self.wake.as_ref().map(|(at, _)| *at) {
            self.wake = wake.map(|at| {
                let wait = (at - self.now + 50).clamp(0, i32::MAX as i64) as u64;
                let task = cx.spawn(async move |this, cx| {
                    cx.background_executor()
                        .timer(Duration::from_millis(wait))
                        .await;
                    let _ = this.update(cx, |this, cx| {
                        this.now = now_ms();
                        this.wake = None;
                        cx.notify()
                    });
                });
                (at, task)
            });
        }
        let checking = state
            .app
            .as_ref()
            .and_then(|app| app.meta.app_update.as_ref())
            .is_some_and(|update| update.state.status == DesktopUpdateStatus::Checking);
        if self.asked && !checking && self.asked_timer.is_none() {
            self.asked_timer = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(Duration::from_secs(6)).await;
                let _ = this.update(cx, |this, cx| {
                    this.asked = false;
                    this.asked_timer = None;
                    cx.notify()
                });
            }));
        }
        self.mark_read(window, cx);
        cx.notify();
    }

    /// The open thread is read while this window is in front, as T3 Code records visits.
    fn mark_read(&mut self, window: &Window, cx: &mut Context<Self>) {
        if !window.is_window_active() {
            return;
        }
        let state = self.state.read(cx);
        let Some(thread) = state.thread() else { return };
        if thread.sidebar.last_read_at.is_some() && !is_unread(thread) && !is_woke(thread, self.now)
        {
            return;
        }
        let key = (
            thread.id.clone(),
            thread.sidebar.turn_ended_at.clone(),
            thread.sidebar.snoozed_until.clone(),
        );
        if self.marked.as_ref() == Some(&key) {
            return;
        }
        state.send(ClientMessage::Sidebar(SidebarClientMessage::MarkRead {
            thread_id: thread.id.clone(),
            read: true,
        }));
        self.marked = Some(key);
    }

    pub(super) fn send(&self, message: ClientMessage, cx: &gpui::App) {
        self.state.read(cx).send(message);
    }

    pub(super) fn lists<'a>(&self, projects: &'a [ProjectSummary]) -> SidebarLists<'a> {
        partition_sidebar(projects, self.now)
    }

    pub fn query(&self, cx: &gpui::App) -> String {
        self.search.read(cx).value().to_string()
    }

    pub(super) fn searching(&self, cx: &gpui::App) -> bool {
        !self.query(cx).trim().is_empty()
    }

    // --- Search ---

    pub(super) fn clear_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.active_result = 0;
    }

    pub(super) fn open_result(
        &mut self,
        thread_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_search(window, cx);
        self.select(thread_id, cx);
    }

    fn open_highlighted(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let thread = self.state.read(cx).app.as_ref().and_then(|app| {
            search_threads(&app.meta.projects, &query)
                .get(self.active_result)
                .map(|thread| thread.id().to_string())
        });
        if let Some(thread) = thread {
            self.open_result(thread, window, cx);
        }
    }

    fn move_result(&mut self, down: bool, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let count = self
            .state
            .read(cx)
            .app
            .as_ref()
            .map_or(0, |app| search_threads(&app.meta.projects, &query).len());
        if count == 0 {
            return;
        }
        self.active_result = if down {
            (self.active_result + 1) % count
        } else {
            (self.active_result + count - 1) % count
        };
        cx.notify();
    }

    // --- Thread actions ---

    pub(super) fn select(&self, thread_id: String, cx: &gpui::App) {
        self.send(ClientMessage::SelectThread { thread_id }, cx);
    }

    /// Settling or snoozing the open thread opens the next listed one, or a new thread in its project.
    fn move_on_from(&mut self, thread_id: &str, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        if state.thread_id() != Some(thread_id) {
            return;
        }
        let Some(app) = state.app.as_ref() else {
            return;
        };
        let lists = self.lists(&app.meta.projects);
        let project = lists
            .order
            .iter()
            .find(|thread| thread.id() == thread_id)
            .map(|thread| thread.project.id.clone())
            .or_else(|| state.project_id().map(String::from));
        match next_thread_after(&lists.order, thread_id) {
            Some(next) => self.select(next.id().to_string(), cx),
            None => {
                if let Some(project) = project {
                    cx.emit(SidebarEvent::NewThread(project));
                }
            }
        }
    }

    pub(super) fn settle(&mut self, thread_id: String, settled: bool, cx: &mut Context<Self>) {
        if settled {
            self.move_on_from(&thread_id, cx);
        }
        self.send(
            ClientMessage::Sidebar(SidebarClientMessage::SettleThread { thread_id, settled }),
            cx,
        );
    }

    pub(super) fn snooze(
        &mut self,
        thread_id: String,
        until: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if until.is_some() {
            self.move_on_from(&thread_id, cx);
        }
        self.send(
            ClientMessage::Sidebar(SidebarClientMessage::SnoozeThread { thread_id, until }),
            cx,
        );
    }

    pub(super) fn pin(&self, thread_id: String, pinned: bool, cx: &gpui::App) {
        self.send(
            ClientMessage::Sidebar(SidebarClientMessage::PinThread { thread_id, pinned }),
            cx,
        );
    }

    pub(super) fn acknowledge_woke(&self, thread_id: String, cx: &gpui::App) {
        self.send(
            ClientMessage::Sidebar(SidebarClientMessage::MarkRead {
                thread_id,
                read: true,
            }),
            cx,
        );
    }

    fn delete(&self, thread_id: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.find_thread(&thread_id, cx).map(|t| t.title.clone()) else {
            return;
        };
        let answer = window.prompt(
            PromptLevel::Warning,
            &format!(
                "Delete \"{title}\"? The chat is gone for good; Resolve and the edit history keep what it changed."
            ),
            None,
            &["OK", "Cancel"],
            cx,
        );
        let state = self.state.clone();
        cx.spawn(async move |_, cx| {
            if answer.await == Ok(0) {
                state.update(cx, |state, _| {
                    state.send(ClientMessage::DeleteThread { thread_id })
                });
            }
        })
        .detach();
    }

    fn find_thread<'a>(&self, thread_id: &str, cx: &'a gpui::App) -> Option<&'a ThreadSummary> {
        self.state
            .read(cx)
            .app
            .as_ref()?
            .meta
            .projects
            .iter()
            .flat_map(|project| &project.threads)
            .find(|thread| thread.id == thread_id)
    }

    /// "New thread": in the open thread's project, else the first listed, else add a project.
    pub(super) fn new_thread(&self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some(app) = state.app.as_ref() else {
            return;
        };
        let project = state
            .project()
            .map(|project| project.id.clone())
            .or_else(|| {
                self.lists(&app.meta.projects)
                    .folders
                    .first()
                    .map(|folder| folder.project.id.clone())
            })
            .or_else(|| app.meta.projects.first().map(|project| project.id.clone()));
        match project {
            Some(project) => cx.emit(SidebarEvent::NewThread(project)),
            None => cx.emit(SidebarEvent::AddProject),
        }
    }

    // --- Renaming ---

    pub(super) fn start_rename(
        &mut self,
        thread_id: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(title) = self.find_thread(&thread_id, cx).map(|t| t.title.clone()) else {
            return;
        };
        let input = cx.new(|cx| InputState::new(window, cx).default_value(title));
        input.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        let id = thread_id.clone();
        cx.subscribe_in(&input, window, move |this, _, event: &InputEvent, _, cx| {
            if matches!(event, InputEvent::PressEnter { .. } | InputEvent::Blur) {
                this.finish_rename(&id, true, cx);
            }
        })
        .detach();
        self.renaming = Some((thread_id, input));
        cx.notify();
    }

    /// Saves (or with `save` false, drops) the title being typed; once.
    pub(super) fn finish_rename(&mut self, thread_id: &str, save: bool, cx: &mut Context<Self>) {
        let Some((id, input)) = self.renaming.take() else {
            return;
        };
        if id != thread_id {
            self.renaming = Some((id, input));
            return;
        }
        let title = input.read(cx).value().trim().to_string();
        let current = self.find_thread(&id, cx).map(|t| t.title.clone());
        if save && !title.is_empty() && Some(&title) != current.as_ref() {
            self.send(
                ClientMessage::RenameThread {
                    thread_id: id,
                    title,
                },
                cx,
            );
        }
        cx.notify();
    }

    // --- Menus ---

    pub fn menu(&self) -> Option<&Entity<ContextMenu<SidebarAction>>> {
        self.menu.as_ref().map(|(menu, _)| menu)
    }

    pub(super) fn open_menu(
        &mut self,
        items: Vec<MenuItem<SidebarAction>>,
        position: Point<Pixels>,
        align_end: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let menu = cx.new(|cx| ContextMenu::new(items, position, align_end, window, cx));
        let subscription = cx.subscribe_in(
            &menu,
            window,
            |this, _, event: &MenuEvent<SidebarAction>, window, cx| match event {
                MenuEvent::Close => this.close_menu(cx),
                MenuEvent::Select(action) => this.run(action.clone(), window, cx),
            },
        );
        self.menu = Some((menu, subscription));
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    pub(super) fn snooze_items(&self, thread_id: &str) -> Vec<MenuItem<SidebarAction>> {
        snooze_presets(Local::now())
            .into_iter()
            .map(|preset| {
                MenuItem::new(
                    format!("snooze:{}", preset.id),
                    preset.label,
                    SidebarAction::Snooze(thread_id.to_string(), Some(preset.snoozed_until)),
                )
                .detail(preset.when_label)
            })
            .collect()
    }

    pub(super) fn thread_menu(&self, thread: &SidebarThread) -> Vec<MenuItem<SidebarAction>> {
        let t = thread.thread;
        let id = t.id.clone();
        let snoozed = time(t.sidebar.snoozed_until.as_deref()).is_some_and(|wake| wake > now_ms())
            && !t.waiting;
        vec![
            if t.sidebar.pinned_at.is_some() {
                MenuItem::new(
                    "unpin",
                    "Unpin thread",
                    SidebarAction::Pin(id.clone(), false),
                )
                .icon(IconName::PinOff)
            } else {
                MenuItem::new("pin", "Pin thread", SidebarAction::Pin(id.clone(), true))
                    .icon(IconName::Pin)
            },
            if t.sidebar.settled_at.is_some() {
                MenuItem::new(
                    "unsettle",
                    "Un-settle thread",
                    SidebarAction::Settle(id.clone(), false),
                )
                .icon(IconName::CircleCheck)
            } else {
                MenuItem::new(
                    "settle",
                    "Settle thread",
                    SidebarAction::Settle(id.clone(), true),
                )
                .icon(IconName::CircleCheck)
            },
            if snoozed {
                MenuItem::new(
                    "unsnooze",
                    "Wake thread",
                    SidebarAction::Snooze(id.clone(), None),
                )
                .icon(IconName::Clock)
            } else {
                MenuItem::submenu("snooze", "Snooze", self.snooze_items(&id))
                    .icon(IconName::Clock)
                    .disabled(t.waiting)
            },
            MenuItem::new("rename", "Rename thread", SidebarAction::Rename(id.clone()))
                .icon(IconName::Pencil)
                .separator(),
            MenuItem::new(
                "mark-unread",
                "Mark unread",
                SidebarAction::MarkUnread(id.clone()),
            )
            .icon(IconName::MailOpen)
            .disabled(t.sidebar.turn_ended_at.is_none() || is_unread(t)),
            MenuItem::submenu(
                "copy",
                "Copy",
                vec![
                    MenuItem::new(
                        "copy-path",
                        "Path",
                        SidebarAction::Copy(thread.project.dir.clone()),
                    )
                    .icon(IconName::Folder),
                    MenuItem::new(
                        "copy-thread-id",
                        "Thread ID",
                        SidebarAction::Copy(id.clone()),
                    )
                    .icon(IconName::Hash),
                ],
            )
            .icon(IconName::Copy)
            .separator(),
            MenuItem::new(
                "project-settings",
                "Project settings",
                SidebarAction::ProjectSettings(thread.project.id.clone()),
            )
            .icon(IconName::Settings),
            if t.archived {
                MenuItem::new(
                    "unarchive",
                    "Unarchive thread",
                    SidebarAction::Archive(id.clone(), false),
                )
                .icon(IconName::ArchiveRestore)
                .separator()
            } else {
                MenuItem::new(
                    "archive",
                    "Archive thread",
                    SidebarAction::Archive(id.clone(), true),
                )
                .icon(IconName::Archive)
                .separator()
                .disabled(t.busy)
            },
            MenuItem::new("delete", "Delete", SidebarAction::Delete(id))
                .icon(IconName::Trash)
                .destructive(),
        ]
    }

    pub(super) fn project_menu(&self, project: &ProjectSummary) -> Vec<MenuItem<SidebarAction>> {
        let id = project.id.clone();
        let change = |pinned: Option<bool>, archived: Option<bool>| {
            SidebarAction::UpdateProject(
                id.clone(),
                ProjectSidebarChanges {
                    icon: None,
                    pinned,
                    archived,
                },
            )
        };
        let mut items = vec![
            MenuItem::new(
                "new-thread",
                "New thread",
                SidebarAction::NewThread(id.clone()),
            )
            .icon(IconName::SquarePen),
            MenuItem::new(
                "copy-path",
                "Copy path",
                SidebarAction::Copy(project.dir.clone()),
            )
            .icon(IconName::Copy),
            if project.sidebar.pinned {
                MenuItem::new("unpin", "Unpin project", change(Some(false), None))
                    .icon(IconName::PinOff)
            } else {
                MenuItem::new("pin", "Pin project", change(Some(true), None)).icon(IconName::Pin)
            },
        ];
        if project.link.is_some() && !project.open {
            items.push(
                MenuItem::new(
                    "open-in-resolve",
                    "Open in Resolve",
                    SidebarAction::OpenInResolve(id.clone()),
                )
                .icon(IconName::MonitorUp),
            );
        }
        items.push(if project.sidebar.archived {
            MenuItem::new("unarchive", "Unarchive project", change(None, Some(false)))
                .icon(IconName::ArchiveRestore)
        } else {
            MenuItem::new("archive", "Archive project", change(None, Some(true)))
                .icon(IconName::Archive)
        });
        items.push(
            MenuItem::new(
                "project-settings",
                "Project settings",
                SidebarAction::ProjectSettings(id),
            )
            .icon(IconName::Settings),
        );
        items
    }

    /// Runs a menu row's action.
    pub fn run(&mut self, action: SidebarAction, window: &mut Window, cx: &mut Context<Self>) {
        match action {
            SidebarAction::Pin(thread, pinned) => self.pin(thread, pinned, cx),
            SidebarAction::Settle(thread, settled) => self.settle(thread, settled, cx),
            SidebarAction::Snooze(thread, until) => self.snooze(thread, until, cx),
            SidebarAction::Rename(thread) => self.start_rename(thread, window, cx),
            SidebarAction::MarkUnread(thread_id) => self.send(
                ClientMessage::Sidebar(SidebarClientMessage::MarkRead {
                    thread_id,
                    read: false,
                }),
                cx,
            ),
            SidebarAction::Copy(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            SidebarAction::ProjectSettings(project) => {
                cx.emit(SidebarEvent::ProjectSettings(project))
            }
            SidebarAction::Archive(thread_id, archived) => self.send(
                ClientMessage::ArchiveThread {
                    thread_id,
                    archived,
                },
                cx,
            ),
            SidebarAction::Delete(thread) => self.delete(thread, window, cx),
            SidebarAction::NewThread(project) => cx.emit(SidebarEvent::NewThread(project)),
            SidebarAction::UpdateProject(project_id, changes) => self.send(
                ClientMessage::UpdateProject {
                    project_id,
                    name: None,
                    changes,
                },
                cx,
            ),
            SidebarAction::OpenInResolve(project_id) => {
                self.send(ClientMessage::OpenInResolve { project_id }, cx)
            }
            SidebarAction::OpenUrl(url) => cx.open_url(&url),
            SidebarAction::SignOut => self.send(ClientMessage::LinkerSignOut, cx),
        }
    }

    // --- Keyboard shortcuts: T3 Code's defaults ---

    /// The workspace passes its key presses on; true when one was a shortcut here.
    pub fn on_key_down(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let keystroke = &event.keystroke;
        let modifiers = keystroke.modifiers;
        if !modifiers.secondary(cx) || modifiers.alt {
            return false;
        }
        let state = self.state.read(cx);
        let Some(app) = state.app.as_ref() else {
            return false;
        };
        let lists = self.lists(&app.meta.projects);
        let current = state.thread_id().map(String::from);
        let key = keystroke.key.as_str();
        if !modifiers.shift && key == "n" {
            self.new_thread(cx);
        } else if !modifiers.shift && key.len() == 1 && ("1"..="9").contains(&key) {
            let index = key.parse::<usize>().unwrap_or(1) - 1;
            let Some(thread) = lists.order.get(index) else {
                return false;
            };
            self.select(thread.id().to_string(), cx);
        } else if modifiers.shift && matches!(key, "[" | "]" | "{" | "}") {
            let Some(current) = current else { return false };
            let Some(index) = lists.order.iter().position(|t| t.id() == current) else {
                return false;
            };
            let next = if matches!(key, "[" | "{") {
                index.checked_sub(1)
            } else {
                Some(index + 1)
            };
            let Some(next) = next.and_then(|next| lists.order.get(next)) else {
                return false;
            };
            self.select(next.id().to_string(), cx);
        } else if modifiers.shift && key == "s" {
            let Some(thread) = state.thread() else {
                return false;
            };
            let (id, settled) = (thread.id.clone(), thread.sidebar.settled_at.is_none());
            self.settle(id, settled, cx);
        } else if modifiers.shift && key == "p" {
            let Some(thread) = state.thread() else {
                return false;
            };
            self.pin(thread.id.clone(), thread.sidebar.pinned_at.is_none(), cx);
        } else {
            return false;
        }
        let _ = window;
        self.hide_hints(cx);
        true
    }

    /// Holding Ctrl (⌘ on a Mac) a moment shows the jump hints.
    pub fn on_modifiers_changed(&mut self, event: &ModifiersChangedEvent, cx: &mut Context<Self>) {
        let modifiers = event.modifiers;
        let alone = modifiers.secondary(cx) && !modifiers.shift && !modifiers.alt;
        if !alone {
            self.hide_hints(cx);
            return;
        }
        if self.hint_timer.is_some() || self.jump_hints {
            return;
        }
        self.hint_timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(200))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.jump_hints = true;
                cx.notify()
            });
        }));
    }

    fn hide_hints(&mut self, cx: &mut Context<Self>) {
        self.hint_timer = None;
        if self.jump_hints {
            self.jump_hints = false;
            cx.notify();
        }
    }

    pub(super) fn jump_label(&self, order: &[SidebarThread], thread_id: &str) -> Option<String> {
        if !self.jump_hints {
            return None;
        }
        let index = order.iter().take(9).position(|t| t.id() == thread_id)?;
        Some(if cfg!(target_os = "macos") {
            format!("⌘{}", index + 1)
        } else {
            format!("Ctrl+{}", index + 1)
        })
    }

    pub(super) fn check_for_updates(&mut self, cx: &mut Context<Self>) {
        self.asked = true;
        self.asked_timer = None;
        self.send(ClientMessage::CheckAppUpdate, cx);
        cx.notify();
    }

    pub(super) fn toggle_folder(&mut self, project_id: &str, cx: &mut Context<Self>) {
        if !self.collapsed.remove(project_id) {
            self.collapsed.insert(project_id.to_string());
        }
        cx.notify();
    }

    pub(super) fn toggle_show_all(&mut self, project_id: &str, cx: &mut Context<Self>) {
        if !self.expanded_folders.remove(project_id) {
            self.expanded_folders.insert(project_id.to_string());
        }
        cx.notify();
    }

    /// A folder's first six, and the open thread when it's further down; and how many are hidden.
    pub(super) fn folder_rows<'a>(
        &self,
        project_id: &str,
        threads: &[SidebarThread<'a>],
        open: Option<&str>,
    ) -> (Vec<SidebarThread<'a>>, usize) {
        if self.expanded_folders.contains(project_id) || threads.len() <= FOLDER_PREVIEW_COUNT {
            return (threads.to_vec(), 0);
        }
        let rows: Vec<_> = threads
            .iter()
            .enumerate()
            .filter(|(index, thread)| *index < FOLDER_PREVIEW_COUNT || Some(thread.id()) == open)
            .map(|(_, thread)| *thread)
            .collect();
        let hidden = threads.len() - rows.len();
        (rows, hidden)
    }

    pub(super) fn set_hovered(&mut self, id: &str, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            self.hovered = Some(id.to_string());
        } else if self.hovered.as_deref() == Some(id) {
            self.hovered = None;
        } else {
            return;
        }
        cx.notify();
    }

    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let app = self.state.read(cx).app.as_ref();
        let version = app.map_or(String::new(), |app| {
            if app.meta.version == "0.0.0" {
                "dev".into()
            } else {
                release_name(&app.meta.version)
            }
        });
        let updates = app.is_some_and(|app| app.meta.app_update.is_some());
        let (accent, foreground) = (theme.accent, theme.foreground);
        title_bar("sidebar-header", px(16.), true, false, window)
            .gap_2()
            .child(LinkerLogo::new(px(20.)))
            .child(
                div()
                    .text_token(TextSize::Sm)
                    .font_weight(FontWeight::MEDIUM)
                    .child("Linker-Edit"),
            )
            .child(if updates {
                no_drag()
                    .ml_auto()
                    .child(
                        div()
                            .id("check-for-updates")
                            .rounded(theme.radius.sm)
                            .px_1()
                            .text_token(TextSize::X2s)
                            .text_color(theme.muted_foreground)
                            .cursor_pointer()
                            .hover(move |style| style.bg(accent).text_color(foreground))
                            .tooltip(|window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new("Check for updates")
                                    .build(window, cx)
                            })
                            .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(cx)))
                            .child(version)
                            .test_support(),
                    )
                    .into_any_element()
            } else {
                div()
                    .id("version")
                    .ml_auto()
                    .text_token(TextSize::X2s)
                    .text_color(theme.muted_foreground)
                    .child(version)
                    .test_support()
                    .into_any_element()
            })
            .into_any_element()
    }

    fn render_search(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let state = self.state.read(cx);
        let Some(app) = state.app.as_ref() else {
            return div().into_any_element();
        };
        let has_projects = !app.meta.projects.is_empty();
        let searching = self.searching(cx);
        let focused = self.search.read(cx).focus_handle(cx).is_focused(window);
        // The project open in Resolve when no Linker-Edit project has it yet.
        let unlinked_open = match &app.meta.resolve {
            ResolveStatus::Connected {
                project: Some(project),
                ..
            } if !app.meta.projects.iter().any(|p| p.open) => Some(project.clone()),
            _ => None,
        };
        let (hover, sidebar_foreground) = (theme.sidebar_row_hover, theme.sidebar_foreground);
        let icon_button = |id: &'static str, icon: IconName, disabled: bool| {
            div()
                .id(id)
                .flex()
                .flex_shrink_0()
                .items_center()
                .justify_center()
                .size_7()
                .rounded(theme.radius.control)
                .map(|this| {
                    if disabled {
                        this.opacity(0.64)
                    } else {
                        this.cursor_pointer().hover(move |style| style.bg(hover))
                    }
                })
                .child(Icon::new(icon).size(px(16.)).text_color(theme.sidebar_icon))
        };
        let search_box = div()
            .id("sidebar-search")
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_2()
            .h_8()
            .px_2()
            .rounded(theme.radius.md)
            .text_token(TextSize::Sm)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.sidebar_muted_foreground)
            .map(|this| {
                if focused {
                    this.bg(hover)
                        .text_color(sidebar_foreground)
                        .border_1()
                        .border_color(theme.ring.opacity(0.7))
                } else {
                    this.hover(move |style| style.bg(hover).text_color(sidebar_foreground))
                }
            })
            .capture_action(cx.listener(|this, _: &MoveDown, _, cx| {
                this.move_result(true, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &MoveUp, _, cx| {
                this.move_result(false, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Escape, window, cx| {
                this.clear_search(window, cx);
                window.blur(cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .child(
                Icon::new(IconName::Search)
                    .size(px(16.))
                    .text_color(theme.sidebar_icon),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_color(theme.sidebar_foreground)
                    .child(bare_input(&self.search, TextSize::Sm)),
            )
            .when(searching, |this| {
                let accent = theme.accent;
                this.child(
                    div()
                        .id("clear-search")
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .size_5()
                        .rounded(theme.radius.sm)
                        .text_color(theme.muted_foreground)
                        .cursor_pointer()
                        .hover(move |style| style.bg(accent))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.clear_search(window, cx);
                            this.search.update(cx, |input, cx| input.focus(window, cx));
                            cx.notify();
                        }))
                        .child(Icon::new(IconName::X).size(px(12.)))
                        .test_support(),
                )
            })
            .test_support();
        div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .p(INSET)
            .child(
                div().flex().items_center().gap_1().child(search_box).child(
                    div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .when(has_projects, |this| {
                            this.child(
                                icon_button("add-project", IconName::FolderPlus, false)
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new("Add project")
                                            .build(window, cx)
                                    })
                                    .on_click(
                                        cx.listener(|_, _, _, cx| {
                                            cx.emit(SidebarEvent::AddProject)
                                        }),
                                    )
                                    .test_support(),
                            )
                        })
                        .child(
                            icon_button("new-thread", IconName::SquarePen, !has_projects)
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "New thread (Ctrl+N)",
                                    )
                                    .build(window, cx)
                                })
                                .when(has_projects, |this| {
                                    this.on_click(cx.listener(|this, _, _, cx| this.new_thread(cx)))
                                })
                                .test_support(),
                        ),
                ),
            )
            .when_some(unlinked_open, |this, project| {
                this.child(
                    div()
                        .id("add-open-project")
                        .mt_1()
                        .flex()
                        .items_center()
                        .gap_2()
                        .h_8()
                        .w_full()
                        .px_2()
                        .rounded(theme.radius.md)
                        .text_token(TextSize::Xs)
                        .text_color(theme.sidebar_muted_foreground)
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover).text_color(sidebar_foreground))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Resolve has this project open; add it to Linker-Edit",
                            )
                            .build(window, cx)
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.send(
                                ClientMessage::AddProject {
                                    source: linker_edit_protocol::ProjectSource::Open,
                                },
                                cx,
                            )
                        }))
                        .child(
                            Icon::new(IconName::Plus)
                                .size(px(16.))
                                .text_color(theme.sidebar_icon),
                        )
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_w_0()
                                .gap_1()
                                .overflow_hidden()
                                .whitespace_nowrap()
                                .child("Add")
                                .child(
                                    div()
                                        .font_weight(FontWeight::MEDIUM)
                                        .text_color(theme.sidebar_foreground)
                                        .truncate()
                                        .child(project),
                                )
                                .child(div().flex_shrink_0().child(", open in Resolve")),
                        )
                        .test_support(),
                )
            })
            .into_any_element()
    }
}

impl Render for Sidebar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let header = self.render_header(window, cx);
        let search = self.render_search(window, cx);
        let list = self.render_list(cx);
        let footer = self.render_footer(cx);
        let menu = self.menu.as_ref().map(|(menu, _)| menu.clone());
        div()
            .id("sidebar")
            .flex()
            .flex_col()
            .flex_shrink_0()
            .w(SIDEBAR_WIDTH)
            .h_full()
            .border_r_1()
            .border_color(theme.border)
            .bg(theme.sidebar)
            .text_color(theme.sidebar_foreground)
            .child(header)
            .child(self.computer_picker.clone())
            .child(search)
            .child(list)
            .child(footer)
            .children(menu)
            .test_support()
    }
}
