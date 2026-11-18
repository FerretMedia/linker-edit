//! The sidebar's and its dialogs' flows, in a window on a link the test drives.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::{
    AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    point, px, size,
};
use gpui_kit::test::TestWindowExt;
use linker_edit_protocol::{
    ClientMessage, CollectionIcon, CollectionKind, CollectionMessage, CollectionSummary,
    ComputerVia, Computers, FolderEntry, FolderListing, IconColor, OrError, ProjectIcon,
    ProjectSidebarChanges, ProjectSource, ProjectSummary, ServerMessage, SidebarClientMessage,
    ThreadSummary,
};

use crate::sidebar::{iso, now_ms};
use crate::state::{AppState, Auth, Link, LinkEvent, ServerAddress, ServerEnd};
use crate::views::collections_settings::CollectionEditor;
use crate::views::computer_picker::{ComputerPicker, Connect, Http};
use crate::workspace::{OpenDialog, Workspace};

struct Server {
    events: UnboundedSender<LinkEvent>,
    commands: UnboundedReceiver<ClientMessage>,
}

impl Server {
    fn new(end: ServerEnd) -> Self {
        Server {
            events: end.events,
            commands: end.commands,
        }
    }

    fn message(&self, message: ServerMessage) {
        self.events
            .unbounded_send(LinkEvent::Message(message))
            .unwrap();
    }

    /// What the window sent since last asked.
    fn received(&mut self) -> Vec<ClientMessage> {
        std::iter::from_fn(|| self.commands.try_recv().ok()).collect()
    }

    fn sidebar_messages(&mut self) -> Vec<ClientMessage> {
        self.received()
            .into_iter()
            .filter(|message| {
                !matches!(
                    message,
                    ClientMessage::Sidebar(SidebarClientMessage::MarkRead { read: true, .. })
                        | ClientMessage::RefreshResolve
                )
            })
            .collect()
    }
}

fn ago(minutes: i64) -> Option<String> {
    Some(iso(now_ms() - minutes * 60_000))
}

fn thread(id: &str, title: &str, edit: impl FnOnce(&mut ThreadSummary)) -> ThreadSummary {
    crate::sidebar::tests::thread(id, |thread| {
        thread.title = title.into();
        thread.updated_at = ago(30).unwrap();
        thread.sidebar.created_at = ago(60).unwrap();
        thread.sidebar.last_read_at = ago(1);
        edit(thread)
    })
}

fn project(
    id: &str,
    name: &str,
    threads: Vec<ThreadSummary>,
    edit: impl FnOnce(&mut ProjectSummary),
) -> ProjectSummary {
    crate::sidebar::tests::project(id, threads, |project| {
        project.name = name.into();
        edit(project)
    })
}

/// Two projects: Promo with a pinned, two active, a settled and a snoozed thread; Wedding with one.
fn projects() -> Vec<ProjectSummary> {
    vec![
        project(
            "promo",
            "Promo",
            vec![
                thread("pinned", "Color pass", |t| t.sidebar.pinned_at = ago(5)),
                thread("rough", "Rough cut", |t| t.sidebar.last_sent_at = ago(2)),
                thread("captions", "Captions", |t| t.sidebar.last_sent_at = ago(3)),
                thread("done", "Old notes", |t| t.sidebar.settled_at = ago(10)),
                thread("later", "Music later", |t| {
                    t.sidebar.snoozed_until = ago(-60)
                }),
            ],
            |_| {},
        ),
        project(
            "wedding",
            "Wedding Jan",
            vec![thread("dance", "First dance", |t| {
                t.sidebar.last_sent_at = ago(20)
            })],
            |_| {},
        ),
    ]
}

fn snapshot() -> linker_edit_protocol::AppState {
    let mut state = crate::state::tests::snapshot();
    state.meta.projects = projects();
    state.meta.project_id = Some("promo".into());
    state.meta.thread_id = Some("rough".into());
    state
}

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>, Server) {
    cx.update(gpui_kit::init);
    let (link, end) = Link::pair();
    let root = Rc::new(RefCell::new(None));
    let kept = root.clone();
    let window = cx.update(|cx| {
        let state = AppState::new(link, cx);
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
        .unwrap()
        .0
    });
    let server = Server::new(end);
    server.events.unbounded_send(LinkEvent::Connected).unwrap();
    server.message(ServerMessage::Snapshot {
        state: Box::new(snapshot()),
    });
    cx.run_until_parked();
    let workspace = root.borrow_mut().take().unwrap();
    (window, workspace, server)
}

fn render<R>(
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
enum Do {
    Click(&'static str),
    ClickAt(&'static str, usize),
    RightClick(&'static str),
    DoubleClick(&'static str),
    Hover(&'static str),
    Press(&'static str),
    Input(&'static str),
}

fn steps(cx: &mut TestAppContext, window: AnyWindowHandle, steps: &[Do]) {
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
fn lists_pinned_threads_project_folders_and_the_shelves(cx: &mut TestAppContext) {
    let (window, _, _server) = open(cx);
    render(cx, window, |window, cx| {
        // Pinned on top, then each project's active threads, newest first.
        let pinned = window.find("thread-pinned").bounds();
        let promo = window.find("project-folder-promo").bounds();
        let rough = window.find("thread-rough").bounds();
        let captions = window.find("thread-captions").bounds();
        let wedding = window.find("project-folder-wedding").bounds();
        assert!(pinned.origin.y < promo.origin.y);
        assert!(rough.origin.y < captions.origin.y && captions.origin.y < wedding.origin.y);
        // Rows are T3 Code's: h-9 in a folder.
        assert_eq!(rough.size.height, px(36.));
        // Settled and snoozed threads wait on their shelves, closed.
        assert!(window.try_find("thread-done").is_none());
        assert!(window.try_find("thread-later").is_none());
        window.click("sidebar-settled-shelf-toggle", cx);
        assert!(window.try_find("thread-done").is_some());
        window.click("sidebar-snoozed-shelf-toggle", cx);
        assert!(window.try_find("thread-later").is_some());

        // A closed folder hides its threads.
        window.click("project-folder-wedding", cx);
        assert!(window.try_find("thread-dance").is_none());
        window.click("project-folder-wedding", cx);
        assert!(window.try_find("thread-dance").is_some());
    });
}

#[gpui::test]
fn a_rows_buttons_settle_snooze_and_open_it(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    steps(
        cx,
        window,
        &[
            Do::Click("thread-captions"),
            Do::Hover("thread-captions"),
            Do::Click("settle-captions"),
            Do::Hover("thread-pinned"),
            Do::Click("snooze-pinned"),
        ],
    );
    let menu = workspace.read_with(cx, |workspace, cx| {
        workspace.sidebar().read(cx).menu().cloned()
    });
    assert!(menu.is_some(), "the snooze button opens the presets");
    render(cx, window, |window, _| {
        assert_eq!(window.find_all("context-menu").len(), 1);
    });
    let preset = menu
        .unwrap()
        .read_with(cx, |menu, _| menu.items()[0].id.clone());
    render(cx, window, |window, cx| {
        window.click(gpui::SharedString::from(format!("menu-{preset}")), cx)
    });
    let sent = server.sidebar_messages();
    assert_eq!(
        sent[0],
        ClientMessage::SelectThread {
            thread_id: "captions".into()
        }
    );
    assert_eq!(
        sent[1],
        ClientMessage::Sidebar(SidebarClientMessage::SettleThread {
            thread_id: "captions".into(),
            settled: true
        })
    );
    assert!(matches!(
        &sent[2],
        ClientMessage::Sidebar(SidebarClientMessage::SnoozeThread { thread_id, until: Some(_) })
            if thread_id == "pinned"
    ));
}

#[gpui::test]
fn settling_the_open_thread_opens_the_next_one(cx: &mut TestAppContext) {
    let (window, _, mut server) = open(cx);
    steps(
        cx,
        window,
        &[Do::Hover("thread-rough"), Do::Click("settle-rough")],
    );
    assert_eq!(
        server.sidebar_messages(),
        [
            ClientMessage::SelectThread {
                thread_id: "captions".into()
            },
            ClientMessage::Sidebar(SidebarClientMessage::SettleThread {
                thread_id: "rough".into(),
                settled: true
            }),
        ]
    );
}

#[gpui::test]
fn the_thread_menu_pins_copies_archives_and_deletes(cx: &mut TestAppContext) {
    let (window, _, mut server) = open(cx);
    let menu = |cx: &mut TestAppContext, item: &'static str| {
        steps(
            cx,
            window,
            &[Do::RightClick("thread-captions"), Do::Click(item)],
        )
    };
    menu(cx, "menu-pin");
    menu(cx, "menu-archive");
    // Copy's submenu opens on hover.
    steps(
        cx,
        window,
        &[
            Do::RightClick("thread-captions"),
            Do::Hover("menu-copy"),
            Do::Click("menu-copy-path"),
        ],
    );
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some("/projects/promo".into())
    );
    menu(cx, "menu-delete");
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("OK");
    cx.run_until_parked();
    assert_eq!(
        server.sidebar_messages(),
        [
            ClientMessage::Sidebar(SidebarClientMessage::PinThread {
                thread_id: "captions".into(),
                pinned: true
            }),
            ClientMessage::ArchiveThread {
                thread_id: "captions".into(),
                archived: true
            },
            ClientMessage::DeleteThread {
                thread_id: "captions".into()
            },
        ]
    );
    // Escape closes the menu without doing anything.
    steps(
        cx,
        window,
        &[Do::RightClick("thread-captions"), Do::Press("escape")],
    );
    render(cx, window, |window, _| {
        assert!(window.try_find("context-menu").is_none())
    });
    assert!(server.sidebar_messages().is_empty());
}

#[gpui::test]
fn the_project_menu_pins_archives_and_opens_its_settings(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    steps(
        cx,
        window,
        &[
            Do::RightClick("project-folder-wedding"),
            Do::Click("menu-pin"),
            Do::RightClick("project-folder-wedding"),
            Do::Click("menu-archive"),
            Do::RightClick("project-folder-wedding"),
            Do::Click("menu-project-settings"),
        ],
    );
    let change = |pinned, archived| ClientMessage::UpdateProject {
        project_id: "wedding".into(),
        name: None,
        changes: ProjectSidebarChanges {
            icon: None,
            pinned,
            archived,
        },
    };
    assert_eq!(
        server.sidebar_messages(),
        [change(Some(true), None), change(None, Some(true))]
    );
    assert!(workspace.read_with(cx, |workspace, _| matches!(
        workspace.dialog(),
        Some(OpenDialog::ProjectSettings(_))
    )));
}

#[gpui::test]
fn renames_a_thread_in_place(cx: &mut TestAppContext) {
    let (window, _, mut server) = open(cx);
    steps(cx, window, &[Do::DoubleClick("thread-captions")]);
    render(cx, window, |window, _| {
        let field = window.find("rename-thread").bounds();
        // The field takes the title's line, not a taller box.
        assert!(field.size.height <= px(24.), "{:?}", field.size);
    });
    steps(
        cx,
        window,
        &[
            Do::Press("ctrl-a"),
            Do::Input("Subtitles"),
            Do::Press("enter"),
        ],
    );
    render(cx, window, |window, _| {
        assert!(window.try_find("rename-thread").is_none())
    });
    let sent = server.sidebar_messages();
    assert!(sent.contains(&ClientMessage::RenameThread {
        thread_id: "captions".into(),
        title: "Subtitles".into()
    }));
    // Escape keeps the old title.
    steps(
        cx,
        window,
        &[
            Do::DoubleClick("thread-captions"),
            Do::Input("Nope"),
            Do::Press("escape"),
        ],
    );
    render(cx, window, |window, _| {
        assert!(window.try_find("rename-thread").is_none())
    });
    assert!(
        !server
            .sidebar_messages()
            .iter()
            .any(|m| matches!(m, ClientMessage::RenameThread { .. }))
    );
}

#[gpui::test]
fn searches_threads_and_opens_one_from_the_keyboard(cx: &mut TestAppContext) {
    let (window, _, mut server) = open(cx);
    steps(cx, window, &[Do::Click("sidebar-search"), Do::Input("c")]);
    render(cx, window, |window, _| {
        // Title starts first: "Captions", "Color pass", then "Rough cut" (a word start)...
        assert!(window.try_find(("sidebar-search-result", 1usize)).is_some());
        assert!(window.try_find("project-folder-promo").is_none());
    });
    steps(cx, window, &[Do::Press("down"), Do::Press("enter")]);
    // Opening one clears the search.
    render(cx, window, |window, _| {
        assert!(window.try_find("project-folder-promo").is_some())
    });
    assert_eq!(
        server.sidebar_messages(),
        [ClientMessage::SelectThread {
            thread_id: "pinned".into()
        }]
    );
    steps(cx, window, &[Do::Click("sidebar-search"), Do::Input("zzz")]);
    render(cx, window, |window, _| {
        assert!(window.try_find("no-threads-found").is_some())
    });
    steps(cx, window, &[Do::Press("escape")]);
    render(cx, window, |window, _| {
        assert!(window.try_find("no-threads-found").is_none())
    });
}

#[gpui::test]
fn keyboard_shortcuts_jump_settle_pin_and_start_threads(cx: &mut TestAppContext) {
    let (window, _, mut server) = open(cx);
    steps(
        cx,
        window,
        &[
            Do::Press("ctrl-2"),
            Do::Press("ctrl-shift-]"),
            Do::Press("ctrl-shift-s"),
            Do::Press("ctrl-shift-p"),
            Do::Press("ctrl-n"),
        ],
    );
    assert_eq!(
        server.sidebar_messages(),
        [
            // The second listed thread; the next after the open one.
            ClientMessage::SelectThread {
                thread_id: "rough".into()
            },
            ClientMessage::SelectThread {
                thread_id: "captions".into()
            },
            ClientMessage::SelectThread {
                thread_id: "captions".into()
            },
            ClientMessage::Sidebar(SidebarClientMessage::SettleThread {
                thread_id: "rough".into(),
                settled: true
            }),
            ClientMessage::Sidebar(SidebarClientMessage::PinThread {
                thread_id: "rough".into(),
                pinned: true
            }),
            ClientMessage::NewThread {
                project_id: "promo".into()
            },
        ]
    );
}

#[gpui::test]
fn adds_a_cloud_project_and_a_project_folder(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    steps(
        cx,
        window,
        &[
            Do::Click("add-project"),
            Do::Click("add-tab-Cloud"),
            Do::Click("cloud-name"),
            Do::Input("Launch"),
            Do::Click("add-cloud"),
        ],
    );
    let sent = server.received();
    assert!(sent.contains(&ClientMessage::LoadLibrary));
    assert!(sent.contains(&ClientMessage::AddProject {
        source: ProjectSource::Cloud {
            name: "Launch".into()
        }
    }));
    assert!(workspace.read_with(cx, |workspace, _| workspace.dialog().is_none()));

    // The folder tab lists the server's folders as you type.
    steps(
        cx,
        window,
        &[Do::Click("add-project"), Do::Click("add-tab-Folder")],
    );
    let home = FolderListing {
        dir: "C:\\Users\\you\\".into(),
        is_project: false,
        entries: vec![
            FolderEntry {
                name: "Videos".into(),
                is_project: false,
            },
            FolderEntry {
                name: "Promo".into(),
                is_project: true,
            },
        ],
        roots: vec!["C:\\".into(), "D:\\".into()],
        home: "C:\\Users\\you\\".into(),
    };
    let answer = |server: &mut Server, listing: &FolderListing| {
        for message in server.received() {
            if let ClientMessage::BrowseFolders { request_id, .. } = message {
                server.message(ServerMessage::Folders {
                    request_id,
                    listing: OrError::Ok(listing.clone()),
                });
            }
        }
    };
    cx.run_until_parked();
    answer(&mut server, &home);
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    answer(&mut server, &home);
    render(cx, window, |window, _| {
        assert!(window.try_find(("folder-row", 0usize)).is_some());
        assert!(window.try_find(("folder-shortcut", 1usize)).is_some());
    });
    // Tab completes "Pr" into the project folder.
    steps(cx, window, &[Do::Input("Pr"), Do::Press("tab")]);
    let promo = FolderListing {
        dir: "C:\\Users\\you\\Promo\\".into(),
        is_project: true,
        entries: vec![],
        roots: home.roots.clone(),
        home: home.home.clone(),
    };
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    answer(&mut server, &promo);
    render(cx, window, |window, cx| window.click("add-folder", cx));
    assert!(server.received().contains(&ClientMessage::AddProject {
        source: ProjectSource::Folder {
            dir: "C:\\Users\\you\\Promo".into()
        }
    }));
}

#[gpui::test]
fn project_settings_save_the_name_icon_and_workspace(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    let mut state = snapshot();
    state.meta.collections = vec![CollectionSummary {
        id: "ferret".into(),
        kind: CollectionKind::Workspace,
        name: "Ferret Media".into(),
        parent_id: None,
        icon: CollectionIcon::Monogram {
            text: String::new(),
            color: IconColor::Blue,
        },
        rules: String::new(),
        rules_file: "RULES.md".into(),
        project_ids: vec![],
        autonomous: false,
        lessons: vec![],
        examples: vec![],
        setup_project_id: None,
        edits: vec![],
    }];
    server.message(ServerMessage::Meta {
        meta: Box::new(state.meta),
    });
    steps(
        cx,
        window,
        &[
            Do::RightClick("project-folder-wedding"),
            Do::Click("menu-project-settings"),
            Do::Click("project-name"),
            Do::Press("ctrl-a"),
            Do::Input("Wedding Jan & Kim"),
            // An emoji from the icon picker.
            Do::Click("choose-project-icon"),
            Do::ClickAt("icon-type", 1),
            Do::ClickAt("emoji", 2),
            Do::Click("icon-save"),
            Do::Click("project-collection"),
            Do::Click("menu-collection-ferret"),
            Do::Click("save-project"),
        ],
    );
    let sent = server.sidebar_messages();
    assert_eq!(
        sent,
        [
            ClientMessage::UpdateProject {
                project_id: "wedding".into(),
                name: Some("Wedding Jan & Kim".into()),
                changes: ProjectSidebarChanges {
                    icon: Some(Some(ProjectIcon::Emoji {
                        emoji: "📹".into()
                    })),
                    pinned: None,
                    archived: None,
                },
            },
            ClientMessage::Collection(CollectionMessage::MoveProject {
                project_id: "wedding".into(),
                collection_id: Some("ferret".into()),
            }),
        ]
    );
    assert!(workspace.read_with(cx, |workspace, _| workspace.dialog().is_none()));

    // Removing asks first.
    steps(
        cx,
        window,
        &[
            Do::RightClick("project-folder-wedding"),
            Do::Click("menu-project-settings"),
            Do::Click("remove-project"),
        ],
    );
    cx.simulate_prompt_answer("OK");
    cx.run_until_parked();
    assert_eq!(
        server.sidebar_messages(),
        [ClientMessage::RemoveProject {
            project_id: "wedding".into()
        }]
    );
}

#[gpui::test]
fn the_footer_refreshes_resolve_and_offers_sign_in(cx: &mut TestAppContext) {
    let (window, _, mut server) = open(cx);
    render(cx, window, |window, cx| {
        window.click("resolve-status", cx);
        assert!(
            window.try_find("linker-sign-in").is_none(),
            "no home server known"
        );
    });
    assert!(server.received().contains(&ClientMessage::RefreshResolve));
}

#[gpui::test]
fn an_empty_app_adds_its_first_project(cx: &mut TestAppContext) {
    let (window, workspace, server) = open(cx);
    let mut empty = snapshot();
    empty.meta.projects.clear();
    empty.meta.project_id = None;
    empty.meta.thread_id = None;
    server.message(ServerMessage::Snapshot {
        state: Box::new(empty),
    });
    render(cx, window, |window, cx| {
        assert!(window.try_find("add-project").is_none());
        window.click("add-project-empty", cx);
    });
    assert!(workspace.read_with(cx, |workspace, _| matches!(
        workspace.dialog(),
        Some(OpenDialog::AddProject(_))
    )));
    steps(cx, window, &[Do::Press("escape")]);
    assert!(workspace.read_with(cx, |workspace, _| workspace.dialog().is_none()));
}

struct FakeHttp;

impl Http for FakeHttp {
    fn computers(&self, _: &ServerAddress) -> Result<Computers, String> {
        Ok(Computers {
            name: "Edit rig".into(),
            version: Some("2026.1115.1200".into()),
            others: vec![
                linker_edit_protocol::Computer {
                    name: "Laptop".into(),
                    url: "https://laptop.tail0.ts.net:8443".into(),
                    version: "2026.1114.900".into(),
                    via: Some(ComputerVia::Tailscale),
                    online: None,
                },
                linker_edit_protocol::Computer {
                    name: "Studio".into(),
                    url: "https://edit-0123456789abcdef.linker.cool".into(),
                    version: "2026.1115.1200".into(),
                    via: Some(ComputerVia::Linker),
                    online: Some(true),
                },
            ],
            home: None,
        })
    }

    fn connect_session(&self, home: &ServerAddress, url: &str) -> Result<String, String> {
        assert_eq!(home.base, "http://127.0.0.1:4317");
        assert_eq!(url, "https://edit-0123456789abcdef.linker.cool");
        Ok("session-1".into())
    }
}

#[gpui::test]
fn opens_another_computer_and_comes_back(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (link, end) = Link::pair();
    let link = link.with_server(ServerAddress::new("http://127.0.0.1:4317", Auth::Local).unwrap());
    let home = Server::new(end);
    let links: Rc<RefCell<Vec<(String, Auth, ServerEnd)>>> = Rc::default();
    let made = links.clone();
    let connect: Connect = Rc::new(move |server: &ServerAddress| {
        let (link, end) = Link::pair();
        made.borrow_mut()
            .push((server.base.clone(), server.auth.clone(), end));
        Ok(link.with_server(server.clone()))
    });
    let picker_cell = Rc::new(RefCell::new(None));
    let kept = picker_cell.clone();
    let window = cx.update(|cx| {
        let state = AppState::new(link, cx);
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            crate::theme::Theme::init(window, cx);
            let picker =
                cx.new(|cx| ComputerPicker::with(state, Arc::new(FakeHttp), connect, window, cx));
            *kept.borrow_mut() = Some(picker.clone());
            picker
        })
        .unwrap()
        .0
    });
    let picker: Entity<ComputerPicker> = picker_cell.borrow_mut().take().unwrap();
    home.events.unbounded_send(LinkEvent::Connected).unwrap();
    home.message(ServerMessage::Snapshot {
        state: Box::new(snapshot()),
    });
    cx.run_until_parked();
    render(cx, window, |window, _| {
        assert!(window.try_find("computer-menu-root").is_some(), "drawn");
    });
    steps(cx, window, &[Do::Click("computer-picker")]);
    // This computer, then the others.
    render(cx, window, |window, _| {
        assert!(window.try_find("computer-0").is_some())
    });
    steps(cx, window, &[Do::Click("computer-1")]);
    assert_eq!(links.borrow()[0].0, "https://laptop.tail0.ts.net:8443");
    assert_eq!(links.borrow()[0].1, Auth::Local);
    assert!(picker.read_with(cx, |picker, _| !picker.is_open()));
    // The laptop answers; the picker shows it, "remote".
    let laptop = Server::new(links.borrow_mut().remove(0).2);
    laptop.events.unbounded_send(LinkEvent::Connected).unwrap();
    laptop.message(ServerMessage::Snapshot {
        state: Box::new(snapshot()),
    });
    render(cx, window, |window, _| {
        assert!(window.try_find("computer-remote").is_some())
    });

    // The Linker Connect computer: a session from this computer's server, as Bearer.
    steps(
        cx,
        window,
        &[Do::Click("computer-picker"), Do::Click("computer-2")],
    );
    cx.run_until_parked();
    assert_eq!(
        links.borrow()[0].0,
        "https://edit-0123456789abcdef.linker.cool"
    );
    assert_eq!(links.borrow()[0].1, Auth::Bearer("session-1".into()));

    // Signed out there: back home, saying why.
    let studio = Server::new(links.borrow_mut().remove(0).2);
    studio.events.unbounded_send(LinkEvent::SignedOut).unwrap();
    cx.run_until_parked();
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("OK");
    assert_eq!(links.borrow().last().unwrap().0, "http://127.0.0.1:4317");
    assert_eq!(links.borrow().last().unwrap().1, Auth::Local);
}

#[gpui::test]
fn the_collection_editor_saves_its_name_switch_and_rules(cx: &mut TestAppContext) {
    cx.update(gpui_kit::init);
    let (link, end) = Link::pair();
    let mut server = Server::new(end);
    let state = cx.update(|cx| AppState::new(link, cx));
    let mut snapshot = snapshot();
    snapshot.meta.collections = vec![CollectionSummary {
        id: "ferret".into(),
        kind: CollectionKind::Workspace,
        name: "Ferret Media".into(),
        parent_id: None,
        icon: CollectionIcon::Lucide {
            name: "mic".into(),
            color: IconColor::Amber,
        },
        rules: "No swearing.".into(),
        rules_file: "RULES.md".into(),
        project_ids: vec![],
        autonomous: false,
        lessons: vec![],
        examples: vec![],
        setup_project_id: None,
        edits: vec![],
    }];
    server.events.unbounded_send(LinkEvent::Connected).unwrap();
    server.message(ServerMessage::Snapshot {
        state: Box::new(snapshot),
    });
    cx.run_until_parked();
    // The settings page makes it for a collection that is there.
    let window = cx.update(|cx| {
        gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
            crate::theme::Theme::init(window, cx);
            cx.new(|cx| CollectionEditor::new(state, "ferret".into(), window, cx))
        })
        .unwrap()
        .0
    });
    steps(
        cx,
        window,
        &[
            Do::Click("collection-name"),
            Do::Press("end"),
            Do::Input(" Group"),
            Do::Press("enter"),
            Do::Click("autonomous"),
            Do::Click("change-icon"),
            Do::Click("color-teal"),
            Do::Click("collection-rules"),
            Do::Press("ctrl-end"),
            Do::Input(" End on the logo."),
            Do::Press("ctrl-s"),
        ],
    );
    let update = |edit: fn(&mut CollectionMessage)| {
        let mut message = CollectionMessage::UpdateCollection {
            id: "ferret".into(),
            name: None,
            icon: None,
            rules: None,
            parent_id: None,
            autonomous: None,
        };
        edit(&mut message);
        ClientMessage::Collection(message)
    };
    assert_eq!(
        server.received(),
        [
            update(
                |m| if let CollectionMessage::UpdateCollection { name, .. } = m {
                    *name = Some("Ferret Media Group".into())
                }
            ),
            update(
                |m| if let CollectionMessage::UpdateCollection { autonomous, .. } = m {
                    *autonomous = Some(true)
                }
            ),
            update(
                |m| if let CollectionMessage::UpdateCollection { icon, .. } = m {
                    *icon = Some(CollectionIcon::Lucide {
                        name: "mic".into(),
                        color: IconColor::Teal,
                    })
                }
            ),
            update(
                |m| if let CollectionMessage::UpdateCollection { rules, .. } = m {
                    *rules = Some("No swearing. End on the logo.".into())
                }
            ),
        ]
    );
}
