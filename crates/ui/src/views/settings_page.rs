//! SettingsPage.svelte: the settings page, which replaces the app while open. A nav of sections
//! on the left (and Library), one section on the right: General (approvals, the agent new
//! threads start with, the theme), Workspaces & series, Resolve (status, "Resolve is on this
//! computer", its MCP server), Transcription (the speech engine, its download with progress and
//! Cancel), Remote access (Linker Connect and Tailscale), Linker account, About & updates (the
//! version, the app's own updates, where files are kept).

use std::time::Duration;

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FontWeight, InteractiveElement,
    IntoElement, ObjectFit, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, StyledImage, Subscription, Window, div, img, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::release_name::{RELEASES_URL, release_name};
use linker_edit_protocol::{
    AppMeta, AppUpdate, ApprovalMode, ClientMessage, DesktopUpdateStatus, LinkerAccount, Provider,
    TranscriptionStatus,
};

use crate::icons::IconName;
use crate::labels::provider_label;
use crate::settings::{SettingsSection, resolve_status_line};
use crate::state::AppState;
use crate::theme::{Choice, TextSize, TextStyled, Theme};
use crate::views::collections_settings::{CollectionsEvent, CollectionsSettings};
use crate::views::linker_connect::LinkerConnectSettings;
use crate::views::remote_access::RemoteAccessSettings;
use crate::views::settings_parts::{
    PageFrame, PathLine, Segmented, SettingsRow, Switch, emphasized, group, linked_text,
    medium_date_time, nav_tab, outline_button, page_title, primary_button, progress_bar,
};

const SIGN_IN: &str = "/api/linker/sign-in";
const COPIED_FOR: Duration = Duration::from_millis(1500);

pub enum SettingsEvent {
    Close,
    Library,
}

pub struct SettingsPage {
    state: Entity<AppState>,
    section: SettingsSection,
    /// What was copied last, for a moment: a path, or "connect" / "remote" for an address.
    copied: Option<String>,
    collections: Option<Entity<CollectionsSettings>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<SettingsEvent> for SettingsPage {}

/// One line on how the app's updates stand; checking says how it went, even with nothing new.
pub fn update_line(app: &AppMeta) -> (String, bool) {
    let Some(update) = &app.app_update else {
        return match &app.update {
            Some(version) => (
                format!(
                    "Linker-Edit {} is out. Stop this one, then run npx linker-edit@latest.",
                    release_name(version)
                ),
                false,
            ),
            None => (
                "Started with npx: a new version shows here when it's out.".into(),
                false,
            ),
        };
    };
    let state = &update.state;
    let version = release_name(
        state
            .downloaded_version
            .as_deref()
            .or(state.available_version.as_deref())
            .unwrap_or(""),
    );
    let text = match state.status {
        DesktopUpdateStatus::Checking => "Looking for updates…".into(),
        DesktopUpdateStatus::Available if state.download_url.is_some() => {
            format!("Linker-Edit {version} is out.")
        }
        DesktopUpdateStatus::Available => format!("Downloading Linker-Edit {version}…"),
        DesktopUpdateStatus::Downloading => format!(
            "Downloading Linker-Edit {version}{}",
            state
                .percent
                .map_or("…".into(), |percent| format!(" · {percent}%"))
        ),
        DesktopUpdateStatus::Downloaded if update.waiting => {
            format!("Linker-Edit restarts into {version} once Resolve is free.")
        }
        DesktopUpdateStatus::Downloaded => {
            format!("Linker-Edit {version} is ready. It installs when you restart or quit.")
        }
        DesktopUpdateStatus::Error => {
            return (
                state
                    .error
                    .clone()
                    .unwrap_or("Couldn't look for updates.".into()),
                true,
            );
        }
        DesktopUpdateStatus::Idle | DesktopUpdateStatus::UpToDate => match &state.checked_at {
            Some(checked) => format!("Up to date. Last checked {}.", medium_date_time(checked)),
            None => "Looks for updates a few seconds after starting, then every hour.".into(),
        },
    };
    (text, false)
}

/// What the speech engine's row says and offers.
#[derive(Debug, PartialEq)]
pub struct TranscriptionLine {
    pub text: String,
    /// Some(None) while it can't say how far it is.
    pub percent: Option<Option<f64>>,
    pub action: Option<TranscriptionAction>,
}

#[derive(Debug, PartialEq)]
pub enum TranscriptionAction {
    Setup(&'static str),
    Cancel,
    Ready,
}

pub fn transcription_line(app: &AppMeta) -> TranscriptionLine {
    let line = |text: &str, action| TranscriptionLine {
        text: text.into(),
        percent: None,
        action,
    };
    match &app.transcription {
        TranscriptionStatus::Preparing { step, percent } => TranscriptionLine {
            text: step.clone(),
            percent: Some(*percent),
            action: Some(TranscriptionAction::Cancel),
        },
        TranscriptionStatus::Transcribing { file, done, total } => TranscriptionLine {
            text: format!(
                "Transcribing {file}{}",
                if *total > 1 {
                    format!(" ({} of {total})", done + 1)
                } else {
                    String::new()
                }
            ),
            percent: None,
            action: Some(TranscriptionAction::Ready),
        },
        TranscriptionStatus::Idle { ready: Some(true) } => line(
            "Installed. Transcripts are made on this computer; nothing is uploaded.",
            Some(TranscriptionAction::Ready),
        ),
        TranscriptionStatus::Idle { .. } if !app.resolve_here => line(
            "Not downloaded here, because Resolve won't be on this computer.",
            Some(TranscriptionAction::Setup("Download anyway")),
        ),
        TranscriptionStatus::Idle { ready: Some(false) } => line(
            "Not downloaded yet. It downloads by itself when Linker-Edit starts: up to 1.6 GB, only once.",
            Some(TranscriptionAction::Setup("Download now")),
        ),
        TranscriptionStatus::Idle { ready: None } => {
            line("Downloads with the first transcription.", None)
        }
    }
}

impl SettingsPage {
    pub fn new(
        state: Entity<AppState>,
        section: SettingsSection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut page = SettingsPage {
            _subscriptions: vec![cx.observe(&state, |_, _, cx| cx.notify())],
            state,
            section: SettingsSection::General,
            copied: None,
            collections: None,
        };
        page.show(section, window, cx);
        page
    }

    pub fn section(&self) -> SettingsSection {
        self.section
    }

    /// Opens a section: what the Svelte sections do when they mount.
    pub fn show(&mut self, section: SettingsSection, window: &mut Window, cx: &mut Context<Self>) {
        self.section = section;
        match section {
            // Linker and Tailscale may have changed since the app last looked.
            SettingsSection::Remote => {
                let state = self.state.read(cx);
                state.send(ClientMessage::RefreshConnect);
                state.send(ClientMessage::RefreshRemoteAccess);
            }
            SettingsSection::Workspaces => {
                let collections =
                    cx.new(|cx| CollectionsSettings::new(self.state.clone(), window, cx));
                self._subscriptions.push(cx.subscribe(
                    &collections,
                    |_, _, event: &CollectionsEvent, cx| match event {
                        CollectionsEvent::SetupOpened => cx.emit(SettingsEvent::Close),
                    },
                ));
                self.collections = Some(collections);
            }
            _ => {}
        }
        if section != SettingsSection::Workspaces {
            self.collections = None;
        }
        cx.notify();
    }

    fn copy(&mut self, key: String, text: String, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text));
        self.copied = Some(key.clone());
        cx.notify();
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(COPIED_FOR).await;
            let _ = this.update(cx, |this, cx| {
                if this.copied.as_ref() == Some(&key) {
                    this.copied = None;
                    cx.notify();
                }
            });
        })
        .detach();
    }

    fn path(&self, id: &str, path: &str, cx: &mut Context<Self>) -> PathLine {
        let text = path.to_string();
        PathLine::new(
            SharedString::from(format!("copy-{id}")),
            path.to_string(),
            self.copied.as_deref() == Some(path),
            cx.listener(move |this, _, _, cx| this.copy(text.clone(), text.clone(), cx)),
        )
    }

    fn send(
        &self,
        message: ClientMessage,
    ) -> impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static {
        let state = self.state.clone();
        move |_, _, cx| state.read(cx).send(message.clone())
    }

    fn general(&self, app: &AppMeta, theme: &Theme, cx: &App) -> Vec<AnyElement> {
        let state = self.state.clone();
        let approvals = Segmented::new(
            "approvals",
            ["Full access", "Supervised"],
            usize::from(app.approval_mode == ApprovalMode::Ask),
            move |index, _, cx| {
                let mode = if index == 0 {
                    ApprovalMode::Auto
                } else {
                    ApprovalMode::Ask
                };
                state.read(cx).send(ClientMessage::SetApprovalMode { mode })
            },
        );
        let state = self.state.clone();
        let providers = Segmented::new(
            "default-provider",
            [
                provider_label(Provider::Claude),
                provider_label(Provider::Codex),
            ],
            usize::from(app.default_provider == Provider::Codex),
            move |index, _, cx| {
                let provider = if index == 0 {
                    Provider::Claude
                } else {
                    Provider::Codex
                };
                state
                    .read(cx)
                    .send(ClientMessage::SetDefaultProvider { provider })
            },
        );
        let choices = [Choice::System, Choice::Light, Choice::Dark];
        let picked = choices
            .iter()
            .position(|&choice| choice == Theme::choice(cx));
        let themes = Segmented::new(
            "theme",
            ["System", "Light", "Dark"],
            picked.unwrap_or(0),
            move |index, window, cx| Theme::choose(choices[index], window, cx),
        );
        vec![
            group(
                "Agents",
                None,
                [
                    SettingsRow::new("Approvals")
                        .description(if app.approval_mode == ApprovalMode::Auto {
                            "Full access: changes run without asking. The project is backed up first, and agents only change their own AI timelines."
                        } else {
                            "Supervised: you Allow or Deny each change to Resolve, and Codex stays in the project's folder."
                        })
                        .control(approvals)
                        .into_any_element(),
                    SettingsRow::new("New threads start with")
                        .description("You can switch agents in any thread, even mid-chat.")
                        .control(providers)
                        .into_any_element(),
                ],
                theme,
            )
            .into_any_element(),
            group(
                "Appearance",
                None,
                [SettingsRow::new("Theme")
                    .description("Kept per window, so each of your computers can have its own.")
                    .control(themes)
                    .into_any_element()],
                theme,
            )
            .into_any_element(),
        ]
    }

    fn resolve(&self, app: &AppMeta, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let status = resolve_status_line(&app.resolve);
        let tone = status.tone.color(theme);
        let detail = if status.detail.is_empty() {
            "Checked every 15 seconds while Linker-Edit is idle.".to_string()
        } else {
            status.detail.clone()
        };
        let state = self.state.clone();
        let mut here = vec![
            SettingsRow::new("Resolve is on this computer")
                .description("Turn off on a computer that only controls your edit computer over remote access. Resolve's setup and the speech engine are then left out here. It turns back on when Resolve is installed.")
                .control(Switch::new("resolve-here", app.resolve_here, move |resolve_here, _, cx| {
                    state.read(cx).send(ClientMessage::SetResolveHere { resolve_here })
                }))
                .into_any_element(),
        ];
        if let Some(setup) = &app.setup {
            // An empty one is none, as `app.setup.resolveMcp ?` reads it.
            let mcp = setup.resolve_mcp.as_deref().filter(|mcp| !mcp.is_empty());
            let mut row = SettingsRow::new("Resolve's MCP server").description(if mcp.is_some() {
                "How Linker-Edit drives Resolve. It comes with Resolve Studio."
            } else {
                "Not found. Install Resolve Studio, or set resolveMcp in the config file to where it is, then restart Linker-Edit."
            });
            if let Some(mcp) = mcp {
                row = row.detail(self.path("resolve-mcp", mcp, cx));
            }
            here.push(row.into_any_element());
        }
        vec![
            group(
                "Status",
                None,
                [SettingsRow::new(status.title.clone())
                    .id("resolve-status-row")
                    .detail(
                        div()
                            .mt_0p5()
                            .flex()
                            .items_start()
                            .gap_2()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child(
                                div()
                                    .mt_1()
                                    .size_2()
                                    .flex_shrink_0()
                                    .rounded_full()
                                    .bg(tone),
                            )
                            .child(div().flex_1().min_w_0().child(detail)),
                    )
                    .control(
                        outline_button("refresh-resolve", theme)
                            .on_click(self.send(ClientMessage::RefreshResolve))
                            .child(Icon::new(IconName::RefreshCw).size(px(14.)))
                            .child("Refresh")
                            .test_support(),
                    )
                    .into_any_element()],
                theme,
            )
            .into_any_element(),
            group(
                "This computer",
                Some(
                    "DaVinci Resolve Studio 21.1.2 or newer, with External scripting set to Local."
                        .into(),
                ),
                here,
                theme,
            )
            .into_any_element(),
        ]
    }

    fn transcription(
        &self,
        app: &AppMeta,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let line = transcription_line(app);
        let ready = line.action == Some(TranscriptionAction::Ready);
        let mut row = SettingsRow::new(if ready { "Ready" } else { "whisper.cpp" })
            .id("speech-engine")
            .description(line.text.clone());
        if let Some(percent) = line.percent {
            row = row.detail(div().mt_2().child(progress_bar(percent, theme)));
            if let Some(percent) = percent {
                row = row.detail(
                    div()
                        .mt_1()
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .child(format!("{percent}%")),
                );
            }
        }
        row = match line.action {
            Some(TranscriptionAction::Setup(label)) => row.control(
                if app.resolve_here {
                    primary_button("transcription-setup", theme)
                } else {
                    outline_button("transcription-setup", theme)
                }
                .on_click(self.send(ClientMessage::TranscriptionSetup))
                .child(label)
                .test_support(),
            ),
            Some(TranscriptionAction::Cancel) => row.control(
                outline_button("transcription-cancel", theme)
                    .on_click(self.send(ClientMessage::TranscriptionCancel))
                    .child("Cancel")
                    .test_support(),
            ),
            Some(TranscriptionAction::Ready) => row.control(
                div()
                    .flex()
                    .items_center()
                    .gap_1p5()
                    .text_token(TextSize::Xs)
                    .text_color(theme.success)
                    .child(Icon::new(IconName::Check).size(px(14.)))
                    .child("Installed"),
            ),
            None => row,
        };
        let mut groups = vec![group(
            "Speech engine",
            Some("whisper.cpp with the large-v3-turbo model: most languages, with a time for every word, on this computer's GPU when it has one.".into()),
            [row.into_any_element()],
            theme,
        )
        .into_any_element()];
        if let Some(setup) = &app.setup {
            groups.push(
                group(
                    "Where it's kept",
                    None,
                    [
                        SettingsRow::new("Transcripts")
                            .description("One text file and one JSON file with word times per clip, kept until the clip changes.")
                            .detail(self.path("transcripts", &setup.folders.transcripts, cx))
                            .into_any_element(),
                        SettingsRow::new("Speech engine and model")
                            .detail(self.path("tools", &setup.folders.tools, cx))
                            .into_any_element(),
                    ],
                    theme,
                )
                .into_any_element(),
            );
        }
        groups
    }

    fn remote(&self, app: &AppMeta, theme: &Theme, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let connect_url = app.connect.url.clone().unwrap_or_default();
        let remote_url = match &app.remote {
            linker_edit_protocol::RemoteAccess::On { url, .. } => url.clone(),
            _ => String::new(),
        };
        vec![
            LinkerConnectSettings::new(
                self.state.clone(),
                self.copied.as_deref() == Some("connect"),
                cx.listener(move |this, _, _, cx| this.copy("connect".into(), connect_url.clone(), cx)),
                cx.listener(|this, _, window, cx| this.show(SettingsSection::Account, window, cx)),
            )
            .into_any_element(),
            div()
                .flex()
                .flex_col()
                .gap_2()
                .child(
                    div()
                        .px_1()
                        .text_token(TextSize::Sm)
                        .font_weight(FontWeight::MEDIUM)
                        .child("Tailscale"),
                )
                .child(
                    div()
                        .px_1()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .child(linked_text(
                            "tailscale-intro",
                            "Or let your other computers use this one over Tailscale (https://tailscale.com). Resolve and the agents keep running here; only devices logged in to your Tailscale account get in.",
                            theme,
                        )),
                )
                .child(
                    div()
                        .rounded(theme.radius.xl)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.card)
                        .p_4()
                        .child(RemoteAccessSettings::new(
                            self.state.clone(),
                            self.copied.as_deref() == Some("remote"),
                            cx.listener(move |this, _, _, cx| {
                                this.copy("remote".into(), remote_url.clone(), cx)
                            }),
                        )),
                )
                .into_any_element(),
        ]
    }

    fn account(
        &self,
        app: &AppMeta,
        on_this_computer: bool,
        theme: &Theme,
        cx: &App,
    ) -> Vec<AnyElement> {
        let sign_in_url = self.state.read(cx).server_url(SIGN_IN);
        let open = |url: Option<String>| {
            move |_: &gpui::ClickEvent, _: &mut Window, cx: &mut App| {
                if let Some(url) = &url {
                    cx.open_url(url)
                }
            }
        };
        let row = match &app.linker {
            LinkerAccount::SignedIn {
                name,
                email,
                picture,
                plan,
                app_url,
            } => {
                let avatar = match picture {
                    Some(picture) => img(picture.clone())
                        .size_9()
                        .flex_shrink_0()
                        .rounded_full()
                        .object_fit(ObjectFit::Cover)
                        .into_any_element(),
                    None => div()
                        .flex()
                        .flex_shrink_0()
                        .items_center()
                        .justify_center()
                        .size_9()
                        .rounded_full()
                        .bg(theme.primary)
                        .text_token(TextSize::Sm)
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.primary_foreground)
                        .child(
                            name.chars()
                                .next()
                                .map(|c| c.to_uppercase().to_string())
                                .unwrap_or_default(),
                        )
                        .into_any_element(),
                };
                let line = format!(
                    "{}{}",
                    plan.as_ref()
                        .map_or("Linker".into(), |plan| format!("Linker {plan}")),
                    email
                        .as_ref()
                        .map(|email| format!(" · {email}"))
                        .unwrap_or_default()
                );
                div()
                    .id("linker-signed-in")
                    .flex()
                    .items_center()
                    .gap_3()
                    .p_4()
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_w_0()
                            .items_center()
                            .gap_3()
                            .child(avatar)
                            .child(
                                div()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .truncate()
                                            .text_token(TextSize::Sm)
                                            .font_weight(FontWeight::MEDIUM)
                                            .child(name.clone()),
                                    )
                                    .child(
                                        div()
                                            .truncate()
                                            .text_token(TextSize::Xs)
                                            .text_color(theme.muted_foreground)
                                            .child(line),
                                    ),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .gap_2()
                            .child(
                                outline_button("open-linker", theme)
                                    .on_click(open(Some(app_url.clone())))
                                    .child(Icon::new(IconName::ExternalLink).size(px(14.)))
                                    .child("Open Linker")
                                    .test_support(),
                            )
                            .child(
                                outline_button("linker-sign-out", theme)
                                    .text_color(theme.destructive)
                                    .on_click(self.send(ClientMessage::LinkerSignOut))
                                    .child(Icon::new(IconName::LogOut).size(px(14.)))
                                    .child("Sign out")
                                    .test_support(),
                            ),
                    )
                    .test_support()
                    .into_any_element()
            }
            LinkerAccount::SigningIn => SettingsRow::new("Finish signing in in your browser")
                .description("Linker-Edit is waiting for Linker to send you back.")
                .control(
                    outline_button("linker-open-again", theme)
                        .on_click(open(sign_in_url))
                        .child("Open it again")
                        .test_support(),
                )
                .control(
                    outline_button("linker-cancel", theme)
                        .on_click(self.send(ClientMessage::LinkerCancelSignIn))
                        .child("Cancel")
                        .test_support(),
                )
                .into_any_element(),
            LinkerAccount::SignedOut { problem } => {
                let mut row = SettingsRow::new("Not signed in").description(if on_this_computer {
                    "Signing in opens Linker in your browser.".to_string()
                } else {
                    format!(
                        "Sign in on {} itself: Linker sends your browser back to it.",
                        app.computer
                    )
                });
                if let Some(problem) = problem {
                    row = row.detail(
                        div()
                            .mt_1()
                            .text_token(TextSize::Xs)
                            .text_color(theme.destructive)
                            .child(problem.clone()),
                    );
                }
                if on_this_computer {
                    row = row.control(
                        primary_button("linker-sign-in", theme)
                            .on_click(open(sign_in_url))
                            .child(Icon::new(IconName::LogIn).size(px(14.)))
                            .child("Sign in with Linker")
                            .test_support(),
                    );
                }
                row.into_any_element()
            }
        };
        vec![group(
            "Linker",
            Some("Optional. Brings your Linker account, and later its projects, to Linker-Edit. Nothing else needs it.".into()),
            [row],
            theme,
        )
        .into_any_element()]
    }

    fn about(
        &self,
        app: &AppMeta,
        on_this_computer: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let (text, error) = update_line(app);
        let update: Option<&AppUpdate> = app.app_update.as_ref();
        let mut version = SettingsRow::new(format!("Version {}", release_name(&app.version)))
            .id("version-row")
            .detail(
                div()
                    .mt_0p5()
                    .text_token(TextSize::Xs)
                    .text_color(if error {
                        theme.destructive
                    } else {
                        theme.muted_foreground
                    })
                    .child(text),
            );
        if let Some(update) = update {
            let state = &update.state;
            if let (DesktopUpdateStatus::Downloading, Some(percent)) = (state.status, state.percent)
            {
                version = version.detail(div().mt_2().child(progress_bar(Some(percent), theme)));
            }
            if !state.notes.is_empty()
                && matches!(
                    state.status,
                    DesktopUpdateStatus::Downloaded | DesktopUpdateStatus::Available
                )
            {
                version = version.detail(
                    div()
                        .mt_2()
                        .pl_4()
                        .flex()
                        .flex_col()
                        .gap_0p5()
                        .text_token(TextSize::Xs)
                        .text_color(theme.muted_foreground)
                        .children(state.notes.iter().map(|note| format!("• {note}"))),
                );
            }
            version = match state.status {
                DesktopUpdateStatus::Downloaded if !update.waiting => version.control(
                    primary_button("install-update", theme)
                        .on_click(self.send(ClientMessage::InstallAppUpdate))
                        .child(if on_this_computer {
                            "Restart to update".to_string()
                        } else {
                            format!("Restart {} to update", app.computer)
                        })
                        .test_support(),
                ),
                DesktopUpdateStatus::Downloaded => version.control(
                    outline_button("cancel-update", theme)
                        .on_click(self.send(ClientMessage::CancelAppUpdate))
                        .child("Not now")
                        .test_support(),
                ),
                DesktopUpdateStatus::Available if state.download_url.is_some() => {
                    let url = state.download_url.clone().unwrap_or_default();
                    version.control(
                        primary_button("download-update", theme)
                            .on_click(move |_, _, cx| cx.open_url(&url))
                            .child("Download it")
                            .test_support(),
                    )
                }
                status => {
                    let busy = matches!(
                        status,
                        DesktopUpdateStatus::Checking | DesktopUpdateStatus::Downloading
                    );
                    version.control(
                        crate::views::settings_parts::disable(
                            outline_button("check-update", theme),
                            busy,
                        )
                        .when(!busy, |this| {
                            this.on_click(self.send(ClientMessage::CheckAppUpdate))
                        })
                        .child("Check for updates")
                        .test_support(),
                    )
                }
            };
        }
        let mut groups = vec![group(
            "Linker-Edit",
            None,
            [
                version.into_any_element(),
                SettingsRow::new("Releases")
                    .description("What changed in each version, and the installers for your other computers.")
                    .control(
                        outline_button("releases", theme)
                            .on_click(|_, _, cx| cx.open_url(RELEASES_URL))
                            .child(Icon::new(IconName::ExternalLink).size(px(14.)))
                            .child("Open")
                            .test_support(),
                    )
                    .into_any_element(),
            ],
            theme,
        )
        .into_any_element()];
        if let Some(setup) = &app.setup {
            let folders = [
                ("projects", "Projects", &setup.folders.projects),
                ("previews", "Preview renders", &setup.folders.previews),
                ("transcripts", "Transcripts", &setup.folders.transcripts),
                ("tools", "Speech engine", &setup.folders.tools),
                ("config", "Config file", &setup.config_file),
            ];
            let rows: Vec<AnyElement> = folders
                .iter()
                .map(|(id, label, path)| {
                    SettingsRow::new(*label)
                        .detail(self.path(id, path, cx))
                        .into_any_element()
                })
                .collect();
            groups.push(
                group(
                    format!("Files on {}", app.computer),
                    Some("To keep them somewhere else, set them in the config file (see linker-edit.config.example.json), then restart Linker-Edit.".into()),
                    rows,
                    theme,
                )
                .into_any_element(),
            );
        }
        groups
    }
}

impl Render for SettingsPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let state = self.state.read(cx);
        let Some(app) = state.app.as_ref().map(|app| app.meta.clone()) else {
            return div().into_any_element();
        };
        let on_this_computer = state.on_this_computer();
        let section = self.section;

        let mut tabs: Vec<AnyElement> = SettingsSection::ALL
            .iter()
            .map(|&entry| {
                nav_tab(
                    SharedString::from(format!("settings-{}", entry.id())),
                    entry.icon(),
                    entry.label(),
                    entry == section,
                    &theme,
                )
                .on_click(cx.listener(move |this, _, window, cx| this.show(entry, window, cx)))
                .test_support()
                .into_any_element()
            })
            .collect();
        tabs.push(
            div()
                .mt_2()
                .pt_2()
                .border_t_1()
                .border_color(theme.border)
                .child(
                    nav_tab(
                        "settings-library",
                        IconName::Library,
                        "Library",
                        false,
                        &theme,
                    )
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Library)))
                    .test_support(),
                )
                .into_any_element(),
        );

        let content = match section {
            SettingsSection::General => self.general(&app, &theme, cx),
            SettingsSection::Workspaces => self
                .collections
                .clone()
                .map(|collections| vec![collections.into_any_element()])
                .unwrap_or_default(),
            SettingsSection::Resolve => self.resolve(&app, &theme, cx),
            SettingsSection::Transcription => self.transcription(&app, &theme, cx),
            SettingsSection::Remote => self.remote(&app, &theme, cx),
            SettingsSection::Account => self.account(&app, on_this_computer, &theme, cx),
            SettingsSection::About => self.about(&app, on_this_computer, &theme, cx),
        };

        PageFrame {
            id: "settings-page",
            on_back: Box::new(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))),
            tabs,
            footer: div()
                .child(emphasized(
                    "Settings of Linker-Edit on ",
                    &app.computer,
                    "",
                    theme.foreground,
                ))
                .into_any_element(),
            header: vec![page_title(section.label()).into_any_element()],
            content,
            max_width: px(672.),
        }
        .render(window, cx)
        .test_support()
        .into_any_element()
    }
}
