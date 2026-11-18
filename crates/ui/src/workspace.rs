//! The root view (+page.svelte's main layout): the sidebar, the chat column with its header, and
//! at most one side panel (preview, history, project file); or the settings or library page in
//! their place; a dialog over it all; "Connecting…" until the server's first snapshot; the toasts.
//! It owns what is only the window's business (which page, panel and dialog are open, the keys);
//! everything else comes from [`AppState`].
//!
//! Each page, panel and dialog is a view made when it opens and dropped when it closes, as the
//! Svelte components mount and unmount, with its subscription beside it.

use std::time::Duration;

use gpui::{
    AnyElement, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, KeyDownEvent, ModifiersChangedEvent, ParentElement, Render,
    SharedString, StatefulInteractiveElement, Styled, Subscription, Task, Window, actions, div,
    prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::{ClientMessage, Tone};

use crate::components::HeaderButton;
use crate::icons::IconName;
use crate::library::LibraryTab;
use crate::media::sound::Sound;
use crate::media::timeline::PreviewTarget;
use crate::settings::SettingsSection;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme, root_text};
use crate::views::add_project::{AddProjectDialog, DialogClosed};
use crate::views::history_panel::{HistoryEvent, HistoryPanel};
use crate::views::library_page::{LibraryEvent, LibraryPage};
use crate::views::project_file_panel::{ProjectFileEvent, ProjectFilePanel};
use crate::views::project_settings::ProjectSettingsDialog;
use crate::views::settings_page::{SettingsEvent, SettingsPage};
use crate::views::{Chat, ChatEvent, Panel, PreviewEvent, PreviewPanel, Sidebar, SidebarEvent};
use crate::window::{WindowControls, no_drag, title_bar};

actions!(
    workspace,
    [
        /// Ctrl+, (Cmd+, on macOS): opens the settings page, or closes it.
        ToggleSettings,
        /// Escape: leaves the settings or library page.
        Back
    ]
);

/// The keys +page.svelte's `onKeydown` listens for. The sidebar's own (T3 Code's Ctrl+N,
/// Ctrl+1…9, …) come through [`Workspace`]'s key handler, as the sidebar decides them.
pub fn bind_keys(cx: &mut gpui::App) {
    cx.bind_keys([
        KeyBinding::new(
            if cfg!(target_os = "macos") {
                "cmd-,"
            } else {
                "ctrl-,"
            },
            ToggleSettings,
            None,
        ),
        KeyBinding::new("escape", Back, None),
    ]);
}

/// A page that replaces the app while open, as `#settings/...` and `#library` do in the browser.
pub enum Page {
    Settings(Entity<SettingsPage>),
    Library(Entity<LibraryPage>),
}

/// A dialog over the window (+page.svelte's `dialog`).
pub enum OpenDialog {
    AddProject(Entity<AddProjectDialog>),
    ProjectSettings(Entity<ProjectSettingsDialog>),
}

/// The open side panel's view.
enum Side {
    Preview(Entity<PreviewPanel>),
    History(Entity<HistoryPanel>),
    /// Made anew for each project, as `{#key app.projectId}` does.
    File(Option<String>, Entity<ProjectFilePanel>),
}

/// How often a library sound is checked for its end.
const SOUND_POLL: Duration = Duration::from_millis(250);

pub struct Workspace {
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    chat: Entity<Chat>,
    panel: Option<Panel>,
    side: Option<(Side, Subscription)>,
    page: Option<(Page, Subscription)>,
    dialog: Option<(OpenDialog, Subscription)>,
    /// What the preview shows, kept while it's closed: the project it belongs to and the version.
    preview_target: (Option<String>, PreviewTarget),
    /// The library page's sound playing, and the watch for its end.
    sound: Option<(Sound, Task<()>)>,
    /// Holds the keyboard when nothing else does, so the shortcuts (Ctrl+, Escape, the
    /// sidebar's) always arrive.
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let chat = cx.new(|cx| Chat::new(state.clone(), window, cx));
        let sidebar = cx.new(|cx| Sidebar::new(state.clone(), window, cx));
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &chat,
                window,
                |this, _, event: &ChatEvent, window, cx| match event {
                    ChatEvent::AddProject => this.add_project(window, cx),
                },
            ),
            cx.subscribe_in(
                &sidebar,
                window,
                |this, _, event: &SidebarEvent, window, cx| this.sidebar_event(event, window, cx),
            ),
            // The user may have switched projects in Resolve meanwhile.
            cx.observe_window_activation(window, |this, window, cx| {
                let state = this.state.read(cx);
                if window.is_window_active() && state.connected {
                    state.send(ClientMessage::RefreshResolve);
                }
            }),
        ];
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        Workspace {
            sidebar,
            chat,
            state,
            panel: None,
            side: None,
            page: None,
            dialog: None,
            preview_target: Default::default(),
            sound: None,
            focus,
            _subscriptions: subscriptions,
        }
    }

    pub fn sidebar(&self) -> &Entity<Sidebar> {
        &self.sidebar
    }

    pub fn chat(&self) -> &Entity<Chat> {
        &self.chat
    }

    pub fn panel(&self) -> Option<Panel> {
        self.panel
    }

    pub fn page(&self) -> Option<&Page> {
        self.page.as_ref().map(|(page, _)| page)
    }

    pub fn dialog(&self) -> Option<&OpenDialog> {
        self.dialog.as_ref().map(|(dialog, _)| dialog)
    }

    /// The preview panel while it's open.
    pub fn preview(&self) -> Option<&Entity<PreviewPanel>> {
        match &self.side {
            Some((Side::Preview(preview), _)) => Some(preview),
            _ => None,
        }
    }

    fn sidebar_event(&mut self, event: &SidebarEvent, window: &mut Window, cx: &mut Context<Self>) {
        match event {
            SidebarEvent::NewThread(project_id) => {
                self.state.read(cx).send(ClientMessage::NewThread {
                    project_id: project_id.clone(),
                });
                self.focus_composer(window, cx);
            }
            SidebarEvent::AddProject => self.add_project(window, cx),
            SidebarEvent::ProjectSettings(project_id) => {
                let state = self.state.clone();
                let project_id = project_id.clone();
                let dialog = cx.new(|cx| ProjectSettingsDialog::new(state, project_id, window, cx));
                let subscription =
                    cx.subscribe_in(&dialog, window, |this, _, _: &DialogClosed, window, cx| {
                        this.close_dialog(window, cx)
                    });
                self.dialog = Some((OpenDialog::ProjectSettings(dialog), subscription));
                cx.notify();
            }
            SidebarEvent::Settings(section) => self.open_settings(*section, window, cx),
        }
    }

    // --- Dialogs ---

    /// AddProjectDialog.svelte.
    pub fn add_project(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.clone();
        let dialog = cx.new(|cx| AddProjectDialog::new(state, window, cx));
        let subscription =
            cx.subscribe_in(&dialog, window, |this, _, _: &DialogClosed, window, cx| {
                this.close_dialog(window, cx)
            });
        self.dialog = Some((OpenDialog::AddProject(dialog), subscription));
        cx.notify();
    }

    fn close_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dialog = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    // --- Pages ---

    /// Opens the settings page at `section`, as `openSettings` does.
    pub fn open_settings(
        &mut self,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(Page::Settings(page)) = self.page() {
            page.update(cx, |page, cx| page.show(section, window, cx));
            return;
        }
        let page = cx.new(|cx| SettingsPage::new(self.state.clone(), section, window, cx));
        let subscription = cx.subscribe_in(
            &page,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Close => this.close_page(window, cx),
                SettingsEvent::Library => this.open_library(LibraryTab::Universal, window, cx),
            },
        );
        self.show_page(Page::Settings(page), subscription, cx);
    }

    /// Opens the library page at `tab`, as `openLibrary` does.
    pub fn open_library(&mut self, tab: LibraryTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Page::Library(page)) = self.page() {
            page.update(cx, |page, cx| page.show(tab, cx));
            return;
        }
        let page = cx.new(|cx| LibraryPage::new(self.state.clone(), tab, window, cx));
        let subscription = cx.subscribe_in(
            &page,
            window,
            |this, page, event: &LibraryEvent, window, cx| match event {
                LibraryEvent::Close => this.close_page(window, cx),
                LibraryEvent::Play(url) => this.play_sound(url, page.clone(), cx),
                LibraryEvent::Stop => this.sound = None,
            },
        );
        self.show_page(Page::Library(page), subscription, cx);
    }

    fn show_page(&mut self, page: Page, subscription: Subscription, cx: &mut Context<Self>) {
        self.sound = None;
        self.page = Some((page, subscription));
        cx.notify();
    }

    /// Escape: closes the library's folder dialog when it is open, else the page.
    fn back(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Page::Library(page)) = self.page()
            && page.read(cx).has_dialog()
        {
            page.update(cx, |page, cx| page.close_dialog(cx));
            return;
        }
        self.close_page(window, cx);
    }

    /// Back to the app, as `closeSettings` does for both pages.
    pub fn close_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = None;
        self.sound = None;
        self.focus.focus(window, cx);
        cx.notify();
    }

    /// The library page's ▶: one sound at a time, as its `<audio>`. The page shows Pause until it
    /// hears that the sound stopped (it ended, or couldn't play).
    fn play_sound(&mut self, url: &str, page: Entity<LibraryPage>, cx: &mut Context<Self>) {
        let sound = self
            .state
            .read(cx)
            .server
            .as_ref()
            .filter(|_| !cfg!(test))
            .and_then(|server| match Sound::play(server, url) {
                Ok(sound) => Some(sound),
                Err(error) => {
                    eprintln!("Couldn't play {url}: {error}");
                    None
                }
            });
        let Some(sound) = sound else {
            self.sound = None;
            page.update(cx, |page, cx| page.stopped(cx));
            return;
        };
        let watch = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(SOUND_POLL).await;
                let ended = this
                    .update(cx, |this, _| {
                        this.sound
                            .as_ref()
                            .is_none_or(|(sound, _)| sound.finished())
                    })
                    .unwrap_or(true);
                if ended {
                    break;
                }
            }
            let _ = this.update(cx, |this, _| this.sound = None);
            page.update(cx, |page, cx| page.stopped(cx));
        });
        self.sound = Some((sound, watch));
    }

    // --- Side panels ---

    /// Opens `panel`, or closes it when it is the open one.
    pub fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        self.panel = (self.panel != Some(panel)).then_some(panel);
        cx.notify();
    }

    /// The version the preview shows: the one picked in the history, while its project is open.
    pub fn preview_target(&self, cx: &gpui::App) -> PreviewTarget {
        let (project, target) = &self.preview_target;
        if project.as_deref() == self.state.read(cx).project_id() {
            target.clone()
        } else {
            PreviewTarget::default()
        }
    }

    /// Opens the preview on `target` of the project in view, as +page.svelte's openPreview.
    pub fn open_preview(&mut self, target: PreviewTarget, cx: &mut Context<Self>) {
        self.preview_target = (self.state.read(cx).project_id().map(str::to_string), target);
        if let Some(preview) = self.preview() {
            let target = self.preview_target.clone();
            preview.update(cx, |preview, cx| preview.set_target(target, cx));
        }
        self.panel = Some(Panel::Preview);
        cx.notify();
    }

    /// Puts `draft` in the composer and the cursor after it.
    fn compose(&mut self, draft: &str, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.chat.read(cx).composer().clone();
        composer.update(cx, |composer, cx| {
            composer.set_draft(draft, window, cx);
            composer.focus(window, cx);
        });
    }

    fn focus_composer(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let composer = self.chat.read(cx).composer().clone();
        composer.update(cx, |composer, cx| composer.focus(window, cx));
    }

    /// The open panel's view: made when it opens, let go of when it closes (the preview stops
    /// its video first), so a closed panel stops asking the server, as the Svelte ones unmount.
    fn side_view(
        &mut self,
        panel: Option<Panel>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let project = self.state.read(cx).project_id().map(str::to_string);
        let keep = match (&self.side, panel) {
            (Some((Side::Preview(_), _)), Some(Panel::Preview))
            | (Some((Side::History(_), _)), Some(Panel::History)) => true,
            (Some((Side::File(for_project, _), _)), Some(Panel::File)) => *for_project == project,
            _ => false,
        };
        if !keep {
            if let Some((Side::Preview(preview), _)) = self.side.take() {
                preview.update(cx, |preview, cx| preview.release(window, cx));
            }
            self.side = panel.map(|panel| self.make_side(panel, project, window, cx));
        }
        Some(match &self.side.as_ref()?.0 {
            Side::Preview(view) => view.clone().into_any_element(),
            Side::History(view) => view.clone().into_any_element(),
            Side::File(_, view) => view.clone().into_any_element(),
        })
    }

    fn make_side(
        &mut self,
        panel: Panel,
        project: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> (Side, Subscription) {
        let state = self.state.clone();
        match panel {
            Panel::Preview => {
                let target = self.preview_target.clone();
                let view = cx.new(|cx| PreviewPanel::new(state, target, cx));
                let subscription = cx.subscribe_in(
                    &view,
                    window,
                    |this, _, event: &PreviewEvent, window, cx| {
                        match event {
                            PreviewEvent::Target(target) => this.open_preview(target.clone(), cx),
                            PreviewEvent::Close => this.panel = None,
                            PreviewEvent::Comment(place) => {
                                // Feedback on the preview starts with where in the video it is.
                                let draft = this.chat.read(cx).composer().read(cx).draft(cx);
                                let text = if draft.trim().is_empty() {
                                    format!("{place}: ")
                                } else {
                                    format!("{}\n{place}: ", draft.trim_end())
                                };
                                this.compose(&text, window, cx);
                            }
                        }
                        cx.notify();
                    },
                );
                (Side::Preview(view), subscription)
            }
            Panel::History => {
                let view = cx.new(|cx| HistoryPanel::new(state, cx));
                let subscription = cx.subscribe_in(
                    &view,
                    window,
                    |this, _, event: &HistoryEvent, window, cx| {
                        match event {
                            HistoryEvent::Close => this.panel = None,
                            HistoryEvent::Restore(draft) => this.compose(draft, window, cx),
                            HistoryEvent::Preview {
                                timeline_id,
                                commit,
                            } => this.open_preview(
                                PreviewTarget {
                                    id: Some(timeline_id.clone()),
                                    commit: Some(commit.clone()),
                                },
                                cx,
                            ),
                        }
                        cx.notify();
                    },
                );
                (Side::History(view), subscription)
            }
            Panel::File => {
                let view = cx.new(|cx| ProjectFilePanel::new(state, window, cx));
                let subscription = cx.subscribe(&view, |this, _, event: &ProjectFileEvent, cx| {
                    match event {
                        ProjectFileEvent::Close => this.panel = None,
                    }
                    cx.notify();
                });
                (Side::File(project, view), subscription)
            }
        }
    }

    // --- Keys ---

    /// The sidebar's shortcuts (T3 Code's), except while a dialog is open.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.dialog.is_some() {
            return;
        }
        if self
            .sidebar
            .update(cx, |sidebar, cx| sidebar.on_key_down(event, window, cx))
        {
            cx.stop_propagation();
        }
    }

    fn on_modifiers_changed(&mut self, event: &ModifiersChangedEvent, cx: &mut Context<Self>) {
        self.sidebar
            .update(cx, |sidebar, cx| sidebar.on_modifiers_changed(event, cx));
    }

    fn toasts(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<AnyElement> {
        let toasts = &self.state.read(cx).toasts;
        if toasts.is_empty() {
            return None;
        }
        let rows = toasts.iter().map(|toast| {
            let id = toast.id;
            div()
                .flex()
                .items_start()
                .gap_2()
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.card)
                .px_3()
                .py_2()
                .text_token(TextSize::Sm)
                .shadow_lg()
                .when(toast.tone == Tone::Error, |this| {
                    this.text_color(theme.destructive)
                })
                .child(div().flex_1().min_w_0().child(toast.text.clone()))
                .child(
                    div()
                        .id(("dismiss-toast", id as usize))
                        .text_color(theme.muted_foreground)
                        .cursor_pointer()
                        .hover(|style| style.text_color(theme.foreground))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.state.update(cx, |state, cx| state.dismiss(id, cx))
                        }))
                        .child(Icon::new(IconName::X).size(px(14.))),
                )
        });
        Some(
            div()
                .id("toasts")
                .absolute()
                .right_4()
                .bottom_4()
                .w_80()
                .flex()
                .flex_col()
                .gap_2()
                .children(rows)
                .test_support()
                .into_any_element(),
        )
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let toasts = self.toasts(&theme, cx);
        if window.focused(cx).is_none() {
            self.focus.focus(window, cx);
        }
        let state = self.state.read(cx);
        // One root for every layout: it holds the keyboard and takes the window's own keys.
        let root = root_text(div(), &theme)
            .id("workspace")
            .relative()
            .size_full()
            .bg(theme.background)
            .track_focus(&self.focus);

        // Until the first snapshot: the whole window is the title bar, with one line in it.
        if state.app.is_none() {
            let text = if state.connected {
                "Loading…"
            } else if state.signed_out {
                "Signed out of this computer. Sign in with Linker again."
            } else {
                "Connecting to the Linker-Edit server…"
            };
            if self.side.is_some() {
                // A relink forgets the project; the panels go with it.
                self.side_view(None, window, cx);
            }
            return root
                .child(
                    title_bar("connecting", px(16.), true, true, window)
                        .size_full()
                        .justify_center()
                        .text_color(theme.muted_foreground)
                        .child(div().id("connecting").child(text).test_support()),
                )
                .child(WindowControls)
                .children(toasts)
                .test_support()
                .into_any_element();
        }

        // The settings or library page replaces the app while open.
        if let Some(page) = self.page() {
            let page = match page {
                Page::Settings(page) => page.clone().into_any_element(),
                Page::Library(page) => page.clone().into_any_element(),
            };
            return root
                .on_action(
                    cx.listener(|this, _: &ToggleSettings, window, cx| match this.page() {
                        Some(Page::Settings(_)) => this.close_page(window, cx),
                        _ => this.open_settings(SettingsSection::General, window, cx),
                    }),
                )
                .on_action(cx.listener(|this, _: &Back, window, cx| this.back(window, cx)))
                .child(page)
                .child(WindowControls)
                .children(toasts)
                .test_support()
                .into_any_element();
        }

        let project = state.project();
        let setup = project.is_some_and(|project| project.setup);
        let panel = self.panel.filter(|_| !setup);
        let rendering = state.rendering();
        let title = SharedString::from(state.chat_title().to_string());
        let badge = project.map(|project| (project.name.clone(), project.dir.clone()));
        let lost = !state.connected;
        let signed_out = state.signed_out;
        let has_project = project.is_some();
        let side = self.side_view(panel, window, cx);

        // The side panels show a Resolve project's timelines; a setup thread has none.
        let toggles = if setup { &[][..] } else { &Panel::ALL[..] };
        let toggles = toggles.iter().map(|&toggle| {
            HeaderButton::new(toggle.id(), toggle.icon(), toggle.label())
                .pressed(panel == Some(toggle))
                .dot(toggle == Panel::Preview && rendering)
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_panel(toggle, cx)))
        });
        let header = title_bar("chat-header", px(20.), false, panel.is_none(), window)
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .id("chat-title")
                    .min_w_0()
                    .truncate()
                    .text_token(TextSize::Sm)
                    .font_weight(FontWeight::MEDIUM)
                    .child(title)
                    .test_support(),
            )
            .when_some(badge, |this, (name, dir)| {
                this.child(
                    div()
                        .id("project-badge")
                        .flex_shrink_0()
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(theme.border)
                        .px_1p5()
                        .py_0p5()
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(dir.clone())
                                .build(window, cx)
                        })
                        .child(name)
                        .test_support(),
                )
            })
            .child(
                no_drag()
                    .ml_auto()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1p5()
                    .child(
                        HeaderButton::new("library", IconName::Library, "Library")
                            .tooltip("Music, sound effects and graphics you reuse")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let tab = if has_project {
                                    LibraryTab::Project
                                } else {
                                    LibraryTab::Universal
                                };
                                this.open_library(tab, window, cx)
                            })),
                    )
                    .children(toggles),
            );

        let main = div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(header)
            .when(lost, |this| {
                this.child(
                    div()
                        .id("lost-connection")
                        .flex()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .px_4()
                        .py_1p5()
                        .border_b_1()
                        .border_color(theme.border)
                        .bg(theme.warning.opacity(0.1))
                        .text_token(TextSize::Xs)
                        .text_color(theme.warning_foreground)
                        .child(Icon::new(IconName::WifiOff).size(px(14.)))
                        .child(if signed_out {
                            "Signed out of this computer. Sign in with Linker again."
                        } else {
                            "Lost connection to the Linker-Edit server. Reconnecting…"
                        })
                        .test_support(),
                )
            })
            .child(self.chat.clone());

        let dialog = self.dialog.as_ref().map(|(dialog, _)| match dialog {
            OpenDialog::AddProject(dialog) => dialog.clone().into_any_element(),
            OpenDialog::ProjectSettings(dialog) => dialog.clone().into_any_element(),
        });
        root.on_key_down(cx.listener(|this, event, window, cx| this.on_key_down(event, window, cx)))
            .on_modifiers_changed(
                cx.listener(|this, event, _, cx| this.on_modifiers_changed(event, cx)),
            )
            .on_action(cx.listener(|this, _: &ToggleSettings, window, cx| {
                this.open_settings(SettingsSection::General, window, cx)
            }))
            .flex()
            .flex_row()
            .child(self.sidebar.clone())
            .child(main)
            .children(side)
            .children(dialog)
            .child(WindowControls)
            .children(toasts)
            .test_support()
            .into_any_element()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
    use gpui::{
        AnyWindowHandle, AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions, point,
        px, size,
    };
    use gpui_kit::test::TestWindowExt;
    use linker_edit_protocol::{
        ApprovalMode, ApprovalRequest, ChatItem, ClientMessage, ServerMessage, ToolStatus,
    };

    use crate::state::{Link, LinkEvent};

    pub(crate) struct Server {
        events: UnboundedSender<LinkEvent>,
        commands: UnboundedReceiver<ClientMessage>,
    }

    impl Server {
        pub(crate) fn new(end: crate::state::ServerEnd) -> Server {
            Server {
                events: end.events,
                commands: end.commands,
            }
        }

        pub(crate) fn send(&self, event: LinkEvent) {
            self.events.unbounded_send(event).unwrap();
        }

        pub(crate) fn message(&self, message: ServerMessage) {
            self.send(LinkEvent::Message(message));
        }

        /// What the window sent since last asked.
        pub(crate) fn received(&mut self) -> Vec<ClientMessage> {
            std::iter::from_fn(|| self.commands.try_recv().ok()).collect()
        }
    }

    /// The window as `run` opens it, at main.ts's size, on a link the test drives.
    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Server) {
        let (window, _, server) = open_workspace(cx);
        (window, server)
    }

    pub(crate) fn open_workspace(
        cx: &mut TestAppContext,
    ) -> (AnyWindowHandle, gpui::Entity<super::Workspace>, Server) {
        cx.update(gpui_kit::init);
        let (link, server) = Link::pair();
        let root = std::rc::Rc::new(std::cell::RefCell::new(None));
        let kept = root.clone();
        let window = cx.update(|cx| {
            let state = crate::AppState::new(link, cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: point(px(0.), px(0.)),
                    size: size(px(1320.), px(860.)),
                })),
                ..crate::window::options(cx)
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let workspace = crate::new_workspace(state, window, cx);
                *kept.borrow_mut() = Some(workspace.clone());
                workspace
            })
            .expect("the window opens")
            .0
        });
        let workspace = root.borrow_mut().take().unwrap();
        (window, workspace, Server::new(server))
    }

    /// Connected, with crates/protocol's snapshot of an empty thread.
    fn open_with_snapshot(cx: &mut TestAppContext) -> (AnyWindowHandle, Server) {
        let (window, server) = open(cx);
        server.send(LinkEvent::Connected);
        server.message(ServerMessage::Snapshot {
            state: Box::new(crate::state::tests::snapshot()),
        });
        cx.run_until_parked();
        (window, server)
    }

    pub(crate) fn render<R>(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut gpui::Window, &mut gpui::App) -> R,
    ) -> R {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .unwrap()
    }

    /// One interaction, each in its own frame, so what it opens is drawn before the next.
    #[derive(Clone, Copy)]
    pub(crate) enum Do {
        Click(&'static str),
        ClickAt(&'static str, usize),
        RightClick(&'static str),
        DoubleClick(&'static str),
        Hover(&'static str),
        Press(&'static str),
        Input(&'static str),
    }

    pub(crate) fn steps(cx: &mut TestAppContext, window: AnyWindowHandle, steps: &[Do]) {
        for step in steps {
            render(cx, window, |window, cx| match *step {
                Do::Click(id) => window.click(id, cx),
                Do::ClickAt(id, index) => window.click((id, index), cx),
                Do::RightClick(id) => window.right_click(id, cx),
                Do::DoubleClick(id) => window.double_click(id, cx),
                Do::Hover(id) => window.hover(id, cx),
                Do::Press(key) => window.press(key, cx),
                Do::Input(text) => window.input(text, cx),
            });
        }
    }

    #[gpui::test]
    fn lays_out_the_columns_and_toggles_a_side_panel(cx: &mut TestAppContext) {
        let (window, _server) = open_with_snapshot(cx);
        render(cx, window, |window, cx| {
            assert_eq!(window.find("sidebar").bounds().size.width, px(256.));
            assert!(window.find("chat").visible());
            assert!(window.try_find("preview-panel").is_none());

            window.click("preview", cx);
            assert_eq!(window.find("preview-panel").bounds().size.width, px(512.));

            window.click("close-panel", cx);
        });
        // The preview says it closed; the workspace takes it away on the next frame.
        render(cx, window, |window, _| {
            assert!(window.try_find("preview-panel").is_none());
        });
    }

    #[gpui::test]
    fn says_connecting_then_so_when_the_link_to_the_server_drops(cx: &mut TestAppContext) {
        let (window, server) = open(cx);
        let shown = |cx: &mut TestAppContext| {
            render(cx, window, |window, _| {
                (
                    window.try_find("connecting").is_some(),
                    window.try_find("lost-connection").is_some(),
                )
            })
        };

        // The first wait is "connecting", not a lost connection.
        assert_eq!(shown(cx), (true, false));
        server.send(LinkEvent::Connected);
        assert_eq!(shown(cx), (true, false));
        server.message(ServerMessage::Snapshot {
            state: Box::new(crate::state::tests::snapshot()),
        });
        assert_eq!(shown(cx), (false, false));
        server.send(LinkEvent::Disconnected);
        assert_eq!(shown(cx), (false, true));
        server.send(LinkEvent::Connected);
        assert_eq!(shown(cx), (false, false));
    }

    #[gpui::test]
    fn a_message_sends_what_the_composer_holds(cx: &mut TestAppContext) {
        let (window, mut server) = open_with_snapshot(cx);
        // A suggestion fills the composer; the send button sends it, trimmed.
        render(cx, window, |window, cx| {
            window.click(("suggestion", 1usize), cx)
        });
        render(cx, window, |window, cx| window.click("send", cx));
        assert_eq!(
            server.received(),
            [ClientMessage::Send {
                text: "What's in the media pool?".into(),
                compact_first: Some(false),
            }]
        );
        // Sent, the composer is empty and the button does nothing.
        render(cx, window, |window, cx| window.click("send", cx));
        assert!(server.received().is_empty());

        // The agent, the approval mode and stop.
        render(cx, window, |window, cx| window.click("approval-mode", cx));
        render(cx, window, |window, cx| window.click("provider", cx));
        render(cx, window, |window, cx| window.click("provider-codex", cx));
        assert_eq!(
            server.received(),
            [
                ClientMessage::SetApprovalMode {
                    mode: ApprovalMode::Ask
                },
                ClientMessage::SetProvider {
                    provider: linker_edit_protocol::Provider::Codex
                },
            ]
        );
        let mut busy = crate::state::tests::snapshot();
        busy.meta.busy = true;
        server.message(ServerMessage::Meta {
            meta: Box::new(busy.meta),
        });
        render(cx, window, |window, cx| {
            assert!(window.try_find("working").is_some());
            window.click("stop", cx)
        });
        assert_eq!(server.received(), [ClientMessage::Stop]);
    }

    #[gpui::test]
    fn spaces_things_as_the_browser_does(cx: &mut TestAppContext) {
        // gpui's px_5, h_7… are rems; with the browser's 16px rem the composer is the Svelte
        // one's 696 × 106 (it was 12.5% smaller with gpui-component's default 14px).
        let (window, _server) = open_with_snapshot(cx);
        render(cx, window, |window, _| {
            let composer = window.find("composer").bounds().size;
            assert_eq!((composer.width, composer.height), (px(696.), px(106.)));
        });
    }

    #[gpui::test]
    fn a_refused_message_goes_back_in_the_composer(cx: &mut TestAppContext) {
        let (window, workspace, server) = open_workspace(cx);
        server.send(LinkEvent::Connected);
        server.message(ServerMessage::Snapshot {
            state: Box::new(crate::state::tests::snapshot()),
        });
        cx.run_until_parked();
        let thread = crate::state::tests::snapshot().meta.thread_id.unwrap();
        server.message(ServerMessage::Refused {
            thread_id: thread,
            text: "Cut the intro".into(),
        });
        render(cx, window, |_, _| {});
        let draft = workspace.read_with(cx, |workspace, cx| {
            let chat = workspace.chat().read(cx);
            chat.composer().read(cx).draft(cx)
        });
        assert_eq!(draft, "Cut the intro");
    }

    #[gpui::test]
    fn approves_and_denies_in_supervised_mode(cx: &mut TestAppContext) {
        let (window, mut server) = open_with_snapshot(cx);
        let mut waiting = crate::state::tests::snapshot();
        waiting.meta.busy = true;
        waiting.meta.approval_mode = ApprovalMode::Ask;
        waiting.meta.approvals = vec![ApprovalRequest {
            id: "a1".into(),
            tool: "mcp__resolve__run_script".into(),
            input: serde_json::json!({ "script": "result = resolve.GetVersionString()" }),
        }];
        server.message(ServerMessage::Meta {
            meta: Box::new(waiting.meta.clone()),
        });
        render(cx, window, |window, cx| {
            // While an approval waits, "Working…" steps aside.
            assert!(window.try_find("working").is_none());
            assert!(window.try_find("approval-a1").is_some());
            window.click("allow", cx);
        });
        assert_eq!(
            server.received(),
            [ClientMessage::Approve {
                id: "a1".into(),
                allow: true
            }]
        );

        waiting.meta.approvals[0] = ApprovalRequest {
            id: "a2".into(),
            tool: "Write".into(),
            input: serde_json::json!({ "file_path": "notes.md" }),
        };
        server.message(ServerMessage::Meta {
            meta: Box::new(waiting.meta.clone()),
        });
        render(cx, window, |window, cx| window.click("deny", cx));
        assert_eq!(
            server.received(),
            [ClientMessage::Approve {
                id: "a2".into(),
                allow: false
            }]
        );

        // The server answers by taking the approval away; the window waits for that.
        waiting.meta.approvals.clear();
        server.message(ServerMessage::Meta {
            meta: Box::new(waiting.meta),
        });
        render(cx, window, |window, _| {
            assert!(window.try_find("approval-a2").is_none());
        });
    }

    #[gpui::test]
    fn a_recorded_session_renders_its_rows(cx: &mut TestAppContext) {
        // The iPad app's recording of the TS server's /ws: a supervised turn with tool calls, an
        // approval, a notice and a reply.
        let (window, mut server) = open(cx);
        server.send(LinkEvent::Connected);
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../protocol/tests/fixtures/ipad/server");
        let mut files: Vec<_> = std::fs::read_dir(&fixtures)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        files.sort();
        let replay = |server: &Server, range: std::ops::Range<usize>| {
            for file in &files[range] {
                let text = std::fs::read_to_string(file).unwrap();
                server.message(serde_json::from_str(&text).unwrap());
            }
        };
        replay(&server, 0..15);
        render(cx, window, |window, cx| {
            assert!(
                window
                    .try_find("approval-00000000-0000-4000-8000-000000000002")
                    .is_some()
            );
            window.click("allow", cx);
        });
        assert_eq!(
            server.received(),
            [ClientMessage::Approve {
                id: "00000000-0000-4000-8000-000000000002".into(),
                allow: true
            }]
        );
        replay(&server, 15..26);
        render(cx, window, |window, cx| {
            assert!(
                window
                    .try_find("user-00000000-0000-4000-8000-000000000001")
                    .is_some()
            );
            assert!(window.try_find("assistant-a1").is_some());
            // Both tool calls in one work group, then the notice and the reply.
            assert!(window.try_find(("work-group", 2usize)).is_some());
            assert!(window.try_find("tool-t2").is_some());
            assert!(
                window
                    .try_find("notice-00000000-0000-4000-8000-000000000003")
                    .is_some()
            );
            assert!(window.try_find("assistant-a2").is_some());
            assert!(
                window
                    .try_find("approval-00000000-0000-4000-8000-000000000002")
                    .is_none()
            );
            window.click("tool-t1", cx);
            assert!(window.try_find("tool-t1-detail").is_some());
        });
    }

    #[gpui::test]
    fn a_long_thread_draws_only_what_is_in_view(cx: &mut TestAppContext) {
        let (window, server) = open_with_snapshot(cx);
        let mut long = crate::state::tests::snapshot();
        long.items = (0..5_000)
            .map(|index| match index % 3 {
                0 => ChatItem::User {
                    id: format!("m{index}"),
                    text: format!("Message {index}"),
                },
                1 => ChatItem::Assistant {
                    id: format!("m{index}"),
                    text: format!("Reply **{index}**\n\n- one\n- two"),
                },
                _ => ChatItem::Tool {
                    id: format!("m{index}"),
                    name: "mcp__linker__list_timelines".into(),
                    input: serde_json::json!({}),
                    status: ToolStatus::Done,
                    output: Some("[]".into()),
                },
            })
            .collect();
        server.message(ServerMessage::Snapshot {
            state: Box::new(long),
        });
        render(cx, window, |window, _| {
            // It opens at the end of the thread, with only a screenful laid out.
            assert!(window.try_find("assistant-m4999").is_some());
            assert!(window.try_find("user-m0").is_none());
            let drawn = (0..5_000)
                .filter(|index| {
                    window
                        .try_find(gpui::SharedString::from(format!("tool-m{index}")))
                        .is_some()
                })
                .count();
            assert!(drawn > 0 && drawn < 100, "{drawn} tool rows drawn");
        });

        // New output keeps the end in view.
        server.message(ServerMessage::Item {
            thread_id: crate::state::tests::snapshot().meta.thread_id.unwrap(),
            item: ChatItem::Assistant {
                id: "m5000".into(),
                text: "Done.".into(),
            },
        });
        render(cx, window, |window, _| {
            assert!(window.try_find("assistant-m5000").is_some());
            assert!(window.try_find("assistant-m4000").is_none());
        });
    }
}
