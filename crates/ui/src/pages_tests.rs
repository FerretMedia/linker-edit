//! The settings and library pages and the history and project file panels, as the window opens
//! them: what they send the server, and what they show of what it sends back.

use gpui::{AnyWindowHandle, AppContext as _, Entity, TestAppContext};
use gpui_kit::test::TestWindowExt;
use linker_edit_protocol::{
    self as protocol, AppUpdate, ApprovalMode, ClientMessage, ClipChange, CollectionIcon,
    CollectionKind, CollectionMessage, CollectionSummary, ConnectDevice, ConnectDeviceKind,
    DesktopUpdateState, DesktopUpdateStatus, FolderListing, HistoryEntry, IconColor,
    LibraryClientMessage, LibraryFile, LibraryFolder, LibraryFolderFields, LibraryFolderStatus,
    LibraryKind, LibraryQuery, LibraryScope, LibraryScopeKind, LibraryServerMessage, LibraryView,
    LinkedComputer, LinkerConnectState, OrError, Provider, ServerMessage, StripTile, TimelineStrip,
    TimelineStripState, TranscriptionStatus,
};

use crate::state::LinkEvent;
use crate::workspace::tests::{Server, open_workspace, render};
use crate::workspace::{Page, PreviewTarget, Workspace};

fn snapshot() -> protocol::AppState {
    crate::state::tests::snapshot()
}

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>, Server) {
    let (window, workspace, server) = open_workspace(cx);
    server.send(LinkEvent::Connected);
    server.message(ServerMessage::Snapshot {
        state: Box::new(snapshot()),
    });
    cx.run_until_parked();
    (window, workspace, server)
}

/// Sends the snapshot's meta with `change` made to it, as the server's patches do.
fn meta(server: &Server, change: impl FnOnce(&mut protocol::AppMeta)) {
    let mut meta = snapshot().meta;
    change(&mut meta);
    server.message(ServerMessage::Meta {
        meta: Box::new(meta),
    });
}

fn click(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) {
    let id = gpui::SharedString::from(id.to_string());
    render(cx, window, |window, cx| window.click(id, cx));
}

fn shown(cx: &mut TestAppContext, window: AnyWindowHandle, id: &str) -> bool {
    let id = gpui::SharedString::from(id.to_string());
    render(cx, window, |window, _| window.try_find(id).is_some())
}

#[gpui::test]
fn settings_open_from_the_sidebar_and_send_what_they_change(cx: &mut TestAppContext) {
    let (window, _, mut server) = open(cx);
    click(cx, window, "settings");
    assert!(shown(cx, window, "settings-page"));
    assert!(!shown(cx, window, "sidebar"));

    // General: approvals and the agent new threads start with.
    click(cx, window, "approvals-1");
    click(cx, window, "default-provider-1");
    // Resolve: refresh, and whether Resolve is on this computer.
    click(cx, window, "settings-resolve");
    click(cx, window, "refresh-resolve");
    click(cx, window, "resolve-here");
    assert_eq!(
        server.received(),
        [
            ClientMessage::SetApprovalMode {
                mode: ApprovalMode::Ask
            },
            ClientMessage::SetDefaultProvider {
                provider: Provider::Codex
            },
            ClientMessage::RefreshResolve,
            ClientMessage::SetResolveHere {
                resolve_here: false
            },
        ]
    );

    // Remote access asks Linker and Tailscale again whenever it opens.
    click(cx, window, "settings-remote");
    assert_eq!(
        server.received(),
        [
            ClientMessage::RefreshConnect,
            ClientMessage::RefreshRemoteAccess
        ]
    );

    // Escape goes back to the app, as does Ctrl+,.
    cx.dispatch_action(window, crate::workspace::Back);
    assert!(!shown(cx, window, "settings-page"));
    assert!(shown(cx, window, "sidebar"));
    cx.dispatch_action(window, crate::workspace::ToggleSettings);
    assert!(shown(cx, window, "settings-page"));
    cx.dispatch_action(window, crate::workspace::ToggleSettings);
    assert!(!shown(cx, window, "settings-page"));
}

#[gpui::test]
fn the_speech_engine_shows_its_download_and_cancels_it(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_settings(crate::settings::SettingsSection::Transcription, window, cx)
        })
    })
    .unwrap();
    meta(&server, |meta| {
        meta.transcription = TranscriptionStatus::Idle { ready: Some(false) }
    });
    click(cx, window, "transcription-setup");
    assert_eq!(server.received(), [ClientMessage::TranscriptionSetup]);

    meta(&server, |meta| {
        meta.transcription = TranscriptionStatus::Preparing {
            step: "Downloading the model".into(),
            percent: Some(40.),
        }
    });
    assert!(!shown(cx, window, "transcription-setup"));
    click(cx, window, "transcription-cancel");
    assert_eq!(server.received(), [ClientMessage::TranscriptionCancel]);
}

fn connect_on(meta: &mut protocol::AppMeta) {
    meta.connect.state = LinkerConnectState::On;
    meta.connect.connected = true;
    meta.connect.url = Some("https://edit-1.linker.cool".into());
    meta.connect.computer_id = "c1".into();
    meta.connect.computers = Some(vec![LinkedComputer {
        id: "c1".into(),
        name: "edit-pc".into(),
        url: "https://edit-1.linker.cool".into(),
        online: Some(true),
        linked_at: "2026-11-01T10:00:00Z".into(),
        last_seen_at: None,
    }]);
    meta.connect.devices = vec![ConnectDevice {
        id: "d1".into(),
        name: "iPad".into(),
        kind: ConnectDeviceKind::Native,
        created_at: "2026-11-02T10:00:00Z".into(),
        last_seen_at: "2026-11-03T10:00:00Z".into(),
    }];
}

#[gpui::test]
fn linker_connect_signs_out_a_device_and_unlinks_after_asking(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    meta(&server, connect_on);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_settings(crate::settings::SettingsSection::Remote, window, cx)
        })
    })
    .unwrap();
    server.received();

    click(cx, window, "sign-out-device-d1");
    assert_eq!(
        server.received(),
        [ClientMessage::ConnectSignOutDevice {
            device_id: "d1".into()
        }]
    );

    // Unlinking asks first; Cancel sends nothing, OK unlinks.
    click(cx, window, "unlink-c1");
    assert!(cx.has_pending_prompt());
    cx.simulate_prompt_answer("Cancel");
    cx.run_until_parked();
    assert!(server.received().is_empty());
    click(cx, window, "unlink-c1");
    cx.simulate_prompt_answer("OK");
    cx.run_until_parked();
    assert_eq!(
        server.received(),
        [ClientMessage::ConnectUnlink {
            computer_id: "c1".into()
        }]
    );

    // On this computer's own server, turning it off doesn't ask.
    click(cx, window, "connect-switch");
    assert!(!cx.has_pending_prompt());
    assert_eq!(
        server.received(),
        [ClientMessage::SetConnect { enabled: false }]
    );
}

fn update(status: DesktopUpdateStatus) -> AppUpdate {
    AppUpdate {
        state: DesktopUpdateState {
            status,
            current_version: "2026.1101.1200".into(),
            available_version: Some("2026.1117.900".into()),
            downloaded_version: Some("2026.1117.900".into()),
            percent: None,
            notes: vec!["Faster previews".into()],
            checked_at: None,
            error: None,
            download_url: None,
            not_applied: None,
        },
        waiting: false,
    }
}

#[gpui::test]
fn about_restarts_into_a_downloaded_update(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    meta(&server, |meta| {
        meta.app_update = Some(update(DesktopUpdateStatus::UpToDate))
    });
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_settings(crate::settings::SettingsSection::About, window, cx)
        })
    })
    .unwrap();
    click(cx, window, "check-update");
    assert_eq!(server.received(), [ClientMessage::CheckAppUpdate]);

    meta(&server, |meta| {
        meta.app_update = Some(update(DesktopUpdateStatus::Downloaded))
    });
    click(cx, window, "install-update");
    let mut waiting = update(DesktopUpdateStatus::Downloaded);
    waiting.waiting = true;
    meta(&server, |meta| meta.app_update = Some(waiting));
    click(cx, window, "cancel-update");
    assert_eq!(
        server.received(),
        [
            ClientMessage::InstallAppUpdate,
            ClientMessage::CancelAppUpdate
        ]
    );
}

#[test]
fn the_update_card_stays_quiet_about_background_checks() {
    use crate::views::app_update_card::update_card_visible;
    assert!(!update_card_visible(
        &update(DesktopUpdateStatus::Idle),
        true
    ));
    assert!(!update_card_visible(
        &update(DesktopUpdateStatus::UpToDate),
        false
    ));
    assert!(update_card_visible(
        &update(DesktopUpdateStatus::UpToDate),
        true
    ));
    let mut offline = update(DesktopUpdateStatus::Error);
    offline.state.available_version = None;
    assert!(!update_card_visible(&offline, false));
    // A failed download of a release it found does show.
    assert!(update_card_visible(
        &update(DesktopUpdateStatus::Error),
        false
    ));
    assert!(update_card_visible(
        &update(DesktopUpdateStatus::Downloading),
        false
    ));
}

fn workspace_collection() -> CollectionSummary {
    CollectionSummary {
        id: "w1".into(),
        kind: CollectionKind::Workspace,
        name: "Ferret Media".into(),
        parent_id: None,
        icon: CollectionIcon::Monogram {
            text: String::new(),
            color: IconColor::Amber,
        },
        rules: String::new(),
        rules_file: "C:\\LinkerEdit\\collections\\ferret-media\\RULES.md".into(),
        project_ids: vec![],
        autonomous: false,
        lessons: vec![],
        examples: vec![],
        setup_project_id: None,
        edits: vec![],
    }
}

#[gpui::test]
fn a_workspace_is_made_picked_and_deleted_after_asking(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_settings(crate::settings::SettingsSection::Workspaces, window, cx)
        })
    })
    .unwrap();
    click(cx, window, "new-workspace");
    cx.simulate_input(window, "Ferret Media");
    click(cx, window, "create");
    assert_eq!(
        server.received(),
        [ClientMessage::Collection(
            CollectionMessage::CreateCollection {
                kind: CollectionKind::Workspace,
                name: "Ferret Media".into(),
                parent_id: Some(None),
                icon: None,
            }
        )]
    );

    // The new one is picked once the server has it.
    meta(&server, |meta| {
        meta.collections = vec![workspace_collection()]
    });
    assert!(shown(cx, window, "collection-editor"));

    click(cx, window, "autonomous");
    let sent = server.received();
    assert!(matches!(
        &sent[0],
        ClientMessage::Collection(CollectionMessage::UpdateCollection {
            autonomous: Some(true),
            ..
        })
    ));
    click(cx, window, "change-icon");
    click(cx, window, "color-blue");
    assert!(matches!(
        &server.received()[..],
        [ClientMessage::Collection(
            CollectionMessage::UpdateCollection {
                icon: Some(CollectionIcon::Monogram {
                    color: IconColor::Blue,
                    ..
                }),
                ..
            }
        )]
    ));

    // Setting it up with AI opens its setup thread behind the settings page.
    click(cx, window, "setup-collection");
    assert_eq!(
        server.received(),
        [ClientMessage::Collection(
            CollectionMessage::SetupCollection { id: "w1".into() }
        )]
    );
    assert!(!shown(cx, window, "settings-page"));

    cx.update_window(window, |_, window, cx| {
        workspace.update(cx, |workspace, cx| {
            workspace.open_settings(crate::settings::SettingsSection::Workspaces, window, cx)
        })
    })
    .unwrap();
    click(cx, window, "collection-w1");
    render(cx, window, |window, cx| {
        let down = gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.), gpui::px(-2000.)));
        window.scroll("settings-page-scroll", down, cx)
    });
    click(cx, window, "delete-collection");
    cx.simulate_prompt_answer("OK");
    cx.run_until_parked();
    assert_eq!(
        server.received(),
        [ClientMessage::Collection(
            CollectionMessage::DeleteCollection { id: "w1".into() }
        )]
    );
    assert!(!shown(cx, window, "collection-editor"));
}

fn history() -> Vec<HistoryEntry> {
    vec![HistoryEntry {
        commit: "36d2983".into(),
        date: "2026-11-18T02:10:34+01:00".into(),
        author: "Codex".into(),
        agent: Some(Provider::Codex),
        subject: "Drop d".into(),
        prompt: Some("Drop d".into()),
        actions: vec!["timeline.remove {\"clip\":\"d\"}".into()],
        changes: vec!["\"AI - Tight\": 1 clip removed (clip d)".into()],
        thread: Some("thread-1".into()),
        timelines: vec![TimelineStrip {
            id: "t2".into(),
            name: "AI - Tight".into(),
            state: TimelineStripState::Changed,
            tiles: vec![
                StripTile::Clip {
                    name: "clip d".into(),
                    media: Some("m-d".into()),
                    image: false,
                    change: Some(ClipChange::Removed),
                    delta: None,
                },
                StripTile::More { count: 2 },
            ],
        }],
    }]
}

#[gpui::test]
fn the_history_asks_for_saves_restores_through_the_composer_and_previews(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    click(cx, window, "history");
    assert_eq!(server.received(), [ClientMessage::LoadHistory]);
    assert!(shown(cx, window, "history-panel"));
    server.message(ServerMessage::History {
        project_id: snapshot().meta.project_id.unwrap(),
        entries: history(),
    });
    assert!(shown(cx, window, "save-36d2983"));
    assert!(shown(cx, window, "history-count"));

    // A reconnect asks again.
    server.send(LinkEvent::Disconnected);
    server.send(LinkEvent::Connected);
    cx.run_until_parked();
    assert_eq!(server.received(), [ClientMessage::LoadHistory]);

    click(cx, window, "actions-36d2983");
    assert!(shown(cx, window, "actions-36d2983-detail"));

    // Restore only writes the request; nothing is sent until the user sends it.
    click(cx, window, "restore-36d2983");
    assert!(server.received().is_empty());
    let draft = workspace.read_with(cx, |workspace, cx| {
        workspace.chat().read(cx).composer().read(cx).draft(cx)
    });
    assert_eq!(
        draft,
        "Restore \"AI - Tight\" as it was in save 36d2983 (\"Drop d\") with restore_timeline."
    );

    click(cx, window, "play-t2");
    let (panel, target) = workspace.read_with(cx, |workspace, cx| {
        (workspace.panel(), workspace.preview_target(cx))
    });
    assert_eq!(panel, Some(crate::views::Panel::Preview));
    assert_eq!(
        target,
        PreviewTarget {
            id: Some("t2".into()),
            commit: Some("36d2983".into())
        }
    );
}

#[gpui::test]
fn the_project_file_follows_the_disk_until_edited_and_saves(cx: &mut TestAppContext) {
    let (window, _, mut server) = open(cx);
    let project_id = snapshot().meta.project_id.unwrap();
    let file = |text: &str| ServerMessage::ProjectFile {
        project_id: project_id.clone(),
        file: "C:\\Projects\\Interview\\PROJECT.md".into(),
        text: text.into(),
    };
    click(cx, window, "file");
    assert_eq!(server.received(), [ClientMessage::LoadProjectFile]);
    assert!(shown(cx, window, "project-file-loading"));
    server.message(file("# Interview\n"));
    assert!(!shown(cx, window, "undo-project-file"));

    click(cx, window, "project-file-text");
    cx.simulate_input(window, "More. ");
    assert!(shown(cx, window, "undo-project-file"));
    // The agent writes the file meanwhile: the draft stays, and it says so.
    server.message(file("# Interview\n\nBy the agent.\n"));
    assert!(shown(cx, window, "project-file-overtaken"));

    click(cx, window, "save-project-file");
    let sent = server.received();
    assert!(matches!(
        &sent[..],
        [ClientMessage::SaveProjectFile { project_id: id, text }]
            if *id == project_id && text.contains("More. ")
    ));
    assert!(!shown(cx, window, "undo-project-file"));
}

fn library_view(scope: LibraryScope) -> LibraryView {
    LibraryView {
        scope,
        scopes: vec![],
        folders: vec![LibraryFolderStatus {
            folder: LibraryFolder {
                id: "f1".into(),
                kind: LibraryKind::Music,
                label: "Music".into(),
                path: "D:\\Music".into(),
                note: String::new(),
            },
            files: 1,
            scanning: None,
            scanned_at: None,
            problem: None,
        }],
        files: vec![LibraryFile {
            folder_id: "f1".into(),
            scope: LibraryScopeKind::Universal,
            kind: LibraryKind::Music,
            rel: "Calm/Piano #1.wav".into(),
            name: "Piano #1.wav".into(),
            path: "D:\\Music\\Calm\\Piano #1.wav".into(),
            size: 10,
            modified: "2026-11-01T10:00:00Z".into(),
            duration: Some(187.4),
            channels: Some(2),
            loudness: Some(-14.2),
            tags: vec![],
            star: false,
            note: String::new(),
            used_in: vec![],
        }],
        total: 1,
    }
}

#[gpui::test]
fn the_library_asks_for_its_library_and_changes_folders_and_files(cx: &mut TestAppContext) {
    let (window, workspace, mut server) = open(cx);
    let project_id = snapshot().meta.project_id.unwrap();
    // From the header, a project's library opens on its own tab.
    click(cx, window, "library");
    let project_scope = LibraryScope::Project {
        project_id: project_id.clone(),
    };
    assert_eq!(
        server.received(),
        [ClientMessage::Library(LibraryClientMessage::LibraryOpen {
            scope: project_scope,
            query: LibraryQuery::default(),
        })]
    );
    click(cx, window, "library-universal");
    assert_eq!(
        server.received(),
        [ClientMessage::Library(LibraryClientMessage::LibraryOpen {
            scope: LibraryScope::Universal,
            query: LibraryQuery::default(),
        })]
    );
    server.message(ServerMessage::LibraryView(
        LibraryServerMessage::LibraryView {
            view: library_view(LibraryScope::Universal),
        },
    ));
    let key = "f1/Calm/Piano #1.wav";
    assert!(shown(cx, window, &format!("file-{key}")));

    click(cx, window, &format!("star-{key}"));
    click(cx, window, "starred");
    let sent = server.received();
    assert_eq!(
        sent[0],
        ClientMessage::Library(LibraryClientMessage::LibraryUpdateFile {
            folder_id: "f1".into(),
            rel: "Calm/Piano #1.wav".into(),
            star: Some(true),
            tags: None,
            note: None,
        })
    );
    assert!(matches!(
        &sent[1],
        ClientMessage::Library(LibraryClientMessage::LibraryOpen { query, .. })
            if query.starred == Some(true)
    ));

    // Playing goes to the player; with none yet, it stops at once.
    click(cx, window, &format!("play-{key}"));
    let playing = workspace.read_with(cx, |workspace, cx| match workspace.page() {
        Some(Page::Library(page)) => page.read(cx).playing().map(String::from),
        _ => panic!("the library is open"),
    });
    assert_eq!(playing, None);

    // Removing a folder asks first.
    click(cx, window, "remove-f1");
    cx.simulate_prompt_answer("OK");
    cx.run_until_parked();
    assert_eq!(
        server.received(),
        [ClientMessage::Library(
            LibraryClientMessage::LibraryRemoveFolder {
                scope: LibraryScope::Universal,
                folder_id: "f1".into(),
            }
        )]
    );

    // Adding one: Add only once the path is a folder on the server's computer.
    click(cx, window, "add-folder");
    assert!(shown(cx, window, "dialog"));
    cx.simulate_input(window, "D:\\Sfx\\");
    cx.executor()
        .advance_clock(std::time::Duration::from_millis(200));
    cx.run_until_parked();
    let asked = server.received();
    let Some(ClientMessage::BrowseFolders { request_id, path }) = asked.last().cloned() else {
        panic!("the path is checked: {asked:?}");
    };
    assert_eq!(path, "D:\\Sfx");
    server.message(ServerMessage::Folders {
        request_id,
        listing: OrError::Ok(FolderListing {
            dir: "D:\\Sfx\\".into(),
            is_project: false,
            entries: vec![],
            roots: vec!["D:\\".into()],
            home: "C:\\Users\\you\\".into(),
        }),
    });
    click(cx, window, "save-folder");
    assert_eq!(
        server.received(),
        [ClientMessage::Library(
            LibraryClientMessage::LibraryAddFolder {
                scope: LibraryScope::Universal,
                folder: LibraryFolderFields {
                    kind: LibraryKind::Music,
                    label: "Sfx".into(),
                    path: "D:\\Sfx".into(),
                    note: String::new(),
                },
            }
        )]
    );
    assert!(!shown(cx, window, "dialog"));

    // Leaving the page stops the server sending it.
    click(cx, window, "back");
    assert!(!shown(cx, window, "library-page"));
    assert_eq!(
        server.received(),
        [ClientMessage::Library(LibraryClientMessage::LibraryClose)]
    );
}
