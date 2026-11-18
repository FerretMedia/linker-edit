//! The root view (+page.svelte's main layout): the sidebar, the chat column with its header,
//! and at most one side panel; or the settings or library page in their place; "Connecting…"
//! until the server's first snapshot; the toasts. It owns what is only the window's business
//! (which page and panel are open); everything else comes from [`AppState`].

use gpui::{
    AnyElement, AppContext, Context, Entity, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, KeyBinding, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Window, actions, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::{ClientMessage, Tone};

use crate::components::HeaderButton;
use crate::icons::IconName;
use crate::library::LibraryTab;
use crate::settings::SettingsSection;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme, root_text};
use crate::views::history_panel::{HistoryEvent, HistoryPanel};
use crate::views::library_page::{LibraryEvent, LibraryPage};
use crate::views::project_file_panel::{ProjectFileEvent, ProjectFilePanel};
use crate::views::settings_page::{SettingsEvent, SettingsPage};
use crate::views::{Chat, ChatEvent, Panel, SidePanel, Sidebar, SidebarEvent};
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

/// The keys +page.svelte's `onKeydown` listens for.
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

/// player.ts's PreviewTarget: a timeline (None: Resolve's current one) as it was in a save
/// (None: as it is). The preview panel's port plays it; History's Play sets it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PreviewTarget {
    pub id: Option<String>,
    pub commit: Option<String>,
}

pub struct Workspace {
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    chat: Entity<Chat>,
    panel: Option<Panel>,
    page: Option<Page>,
    history: Option<Entity<HistoryPanel>>,
    /// PROJECT.md's panel, made anew for each project.
    file: Option<(Option<String>, Entity<ProjectFilePanel>)>,
    /// The version picked in the history, with the project it belongs to.
    preview_target: (Option<String>, PreviewTarget),
    /// Holds the keyboard when nothing else does, so Ctrl+, and Escape always arrive.
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Workspace {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let chat = cx.new(|cx| Chat::new(state.clone(), window, cx));
        let sidebar = cx.new(|cx| Sidebar::new(state.clone(), cx));
        let subscriptions = vec![
            cx.observe(&state, |_, _, cx| cx.notify()),
            cx.subscribe_in(
                &sidebar,
                window,
                |this, _, event: &SidebarEvent, window, cx| match event {
                    SidebarEvent::Settings(section) => this.open_settings(*section, window, cx),
                },
            ),
            // The add-project dialog (AddProjectDialog.svelte) opens here once it is ported.
            cx.subscribe(&chat, |_, _, event: &ChatEvent, _| match event {
                ChatEvent::AddProject => {}
            }),
            // The user may have switched projects in Resolve meanwhile.
            cx.observe_window_activation(window, |this, window, cx| {
                let state = this.state.read(cx);
                if window.is_window_active() && state.connected {
                    state.send(ClientMessage::RefreshResolve);
                }
            }),
        ];
        Workspace {
            sidebar,
            chat,
            state,
            panel: None,
            page: None,
            history: None,
            file: None,
            preview_target: (None, PreviewTarget::default()),
            focus: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    pub fn page(&self) -> Option<&Page> {
        self.page.as_ref()
    }

    /// Opens the settings page at `section`, as `openSettings` does.
    pub fn open_settings(
        &mut self,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(Page::Settings(page)) = &self.page {
            page.update(cx, |page, cx| page.show(section, window, cx));
            return;
        }
        let page = cx.new(|cx| SettingsPage::new(self.state.clone(), section, window, cx));
        self._subscriptions.push(cx.subscribe_in(
            &page,
            window,
            |this, _, event: &SettingsEvent, window, cx| match event {
                SettingsEvent::Close => this.close_page(cx),
                SettingsEvent::Library => this.open_library(LibraryTab::Universal, window, cx),
            },
        ));
        self.page = Some(Page::Settings(page));
        cx.notify();
    }

    /// Opens the library page at `tab`, as `openLibrary` does.
    pub fn open_library(&mut self, tab: LibraryTab, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(Page::Library(page)) = &self.page {
            page.update(cx, |page, cx| page.show(tab, cx));
            return;
        }
        let page = cx.new(|cx| LibraryPage::new(self.state.clone(), tab, window, cx));
        self._subscriptions
            .push(
                cx.subscribe(&page, |this, page, event: &LibraryEvent, cx| match event {
                    LibraryEvent::Close => this.close_page(cx),
                    // The preview's port plugs its player in here. Until then nothing plays, so the
                    // page hears at once that it stopped, as from a failed `audio.play()`.
                    LibraryEvent::Play(_) => page.update(cx, |page, cx| page.stopped(cx)),
                    LibraryEvent::Stop => {}
                }),
            );
        self.page = Some(Page::Library(page));
        cx.notify();
    }

    /// Escape: closes the library's folder dialog when it is open, else the page.
    fn back(&mut self, cx: &mut Context<Self>) {
        if let Some(Page::Library(page)) = &self.page
            && page.read(cx).has_dialog()
        {
            page.update(cx, |page, cx| page.close_dialog(cx));
            return;
        }
        self.close_page(cx);
    }

    /// Back to the app, as `closeSettings` does for both pages.
    pub fn close_page(&mut self, cx: &mut Context<Self>) {
        self.page = None;
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

    /// Opens the preview on a version, as +page.svelte's `openPreview` does.
    pub fn open_preview(&mut self, target: PreviewTarget, cx: &mut Context<Self>) {
        let project = self.state.read(cx).project_id().map(String::from);
        self.preview_target = (project, target);
        self.panel = Some(Panel::Preview);
        cx.notify();
    }

    pub fn panel(&self) -> Option<Panel> {
        self.panel
    }

    pub fn chat(&self) -> &Entity<Chat> {
        &self.chat
    }

    /// Opens `panel`, or closes it when it is the open one.
    pub fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        self.panel = (self.panel != Some(panel)).then_some(panel);
        cx.notify();
    }

    fn history_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<HistoryPanel> {
        if let Some(panel) = &self.history {
            return panel.clone();
        }
        let panel = cx.new(|cx| HistoryPanel::new(self.state.clone(), cx));
        self._subscriptions.push(cx.subscribe_in(
            &panel,
            window,
            |this, _, event: &HistoryEvent, window, cx| {
                match event {
                    HistoryEvent::Close => this.panel = None,
                    HistoryEvent::Restore(draft) => {
                        let composer = this.chat.read(cx).composer().clone();
                        composer.update(cx, |composer, cx| {
                            composer.set_draft(draft, window, cx);
                            composer.focus(window, cx);
                        })
                    }
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
        ));
        self.history = Some(panel.clone());
        panel
    }

    fn file_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ProjectFilePanel> {
        let project = self.state.read(cx).project_id().map(String::from);
        if let Some((for_project, panel)) = &self.file
            && *for_project == project
        {
            return panel.clone();
        }
        let panel = cx.new(|cx| ProjectFilePanel::new(self.state.clone(), window, cx));
        self._subscriptions.push(
            cx.subscribe(&panel, |this, _, event: &ProjectFileEvent, cx| {
                match event {
                    ProjectFileEvent::Close => this.panel = None,
                }
                cx.notify();
            }),
        );
        self.file = Some((project, panel.clone()));
        panel
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

        // Until the first snapshot: the whole window is the title bar, with one line in it.
        if state.app.is_none() {
            let text = if state.connected {
                "Loading…"
            } else if state.signed_out {
                "Signed out of this computer. Sign in with Linker again."
            } else {
                "Connecting to the Linker-Edit server…"
            };
            return root_text(div(), &theme)
                .id("workspace")
                .relative()
                .size_full()
                .bg(theme.background)
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
        if let Some(page) = &self.page {
            let page = match page {
                Page::Settings(page) => page.clone().into_any_element(),
                Page::Library(page) => page.clone().into_any_element(),
            };
            return root_text(div(), &theme)
                .id("workspace")
                .relative()
                .size_full()
                .bg(theme.background)
                .track_focus(&self.focus)
                .on_action(
                    cx.listener(|this, _: &ToggleSettings, window, cx| match &this.page {
                        Some(Page::Settings(_)) => this.close_page(cx),
                        _ => this.open_settings(SettingsSection::General, window, cx),
                    }),
                )
                .on_action(cx.listener(|this, _: &Back, _, cx| this.back(cx)))
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

        // History and the project file are views of their own, dropped while closed as the
        // Svelte ones unmount (so they stop asking the server); the project file is made anew
        // per project, as `{#key app.projectId}` does.
        if panel != Some(Panel::History) {
            self.history = None;
        }
        if panel != Some(Panel::File) {
            self.file = None;
        }
        let side = match panel {
            Some(Panel::History) => Some(self.history_panel(window, cx).into_any_element()),
            Some(Panel::File) => Some(self.file_panel(window, cx).into_any_element()),
            Some(panel) => Some(
                SidePanel::new(
                    panel,
                    cx.listener(|this, _, _, cx| {
                        this.panel = None;
                        cx.notify();
                    }),
                )
                .into_any_element(),
            ),
            None => None,
        };

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

        root_text(div(), &theme)
            .id("workspace")
            .relative()
            .flex()
            .flex_row()
            .size_full()
            .bg(theme.background)
            .track_focus(&self.focus)
            .on_action(cx.listener(|this, _: &ToggleSettings, window, cx| {
                this.open_settings(SettingsSection::General, window, cx)
            }))
            .child(self.sidebar.clone())
            .child(main)
            .children(side)
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
        (
            window,
            workspace,
            Server {
                events: server.events,
                commands: server.commands,
            },
        )
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
