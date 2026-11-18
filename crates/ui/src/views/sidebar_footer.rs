//! sidebar/SidebarFooter.svelte, in T3 Code's SidebarFooter spacing (MIT, see NOTICE), with what it
//! holds: transcription's progress, AppUpdateCard.svelte (the desktop app's own updates) or the
//! npm release, LinkerAccount.svelte (with RowMenu.svelte's menu), Settings, and Resolve's status
//! pill, which refreshes it.

use gpui::{
    AnyElement, Context, FontWeight, Hsla, InteractiveElement, IntoElement, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, div, img, point, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::release_name::{RELEASES_URL, release_name};
use linker_edit_protocol::{
    AppUpdate, ClientMessage, DesktopUpdateStatus, LinkerAccount, ResolveStatus,
    TranscriptionStatus,
};

use crate::icons::IconName;
use crate::state::origin;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::context_menu::MenuItem;
use crate::views::sidebar::{INSET, Sidebar, SidebarAction, SidebarEvent};

/// settings.ts's resolveStatusLine: Resolve's state in one line, as the sidebar and the settings
/// page show it. The tone is the dot's color.
pub struct StatusLine {
    pub tone: Hsla,
    pub title: String,
    pub detail: String,
}

pub fn resolve_status_line(resolve: &ResolveStatus, theme: &Theme) -> StatusLine {
    let line = |tone, title: &str, detail: &str| StatusLine {
        tone,
        title: title.into(),
        detail: detail.into(),
    };
    match resolve {
        ResolveStatus::Checking => line(theme.warning, "Checking Resolve…", ""),
        ResolveStatus::Connected {
            product,
            version,
            project,
            timeline,
            ..
        } => StatusLine {
            tone: theme.success,
            title: format!(
                "{product} {}",
                version.split('.').take(3).collect::<Vec<_>>().join(".")
            ),
            detail: match (project, timeline) {
                (Some(project), Some(timeline)) => format!("{project} · {timeline}"),
                (Some(project), None) => project.clone(),
                (None, _) => "No project open".into(),
            },
        },
        ResolveStatus::NotRunning => line(
            theme.muted_foreground,
            "Resolve is not running",
            "Start Resolve, then refresh.",
        ),
        ResolveStatus::Blocked { hint } => line(theme.warning, "Resolve needs attention", hint),
        ResolveStatus::Unreachable { hint } => {
            line(theme.destructive, "Resolve not reachable", hint)
        }
        ResolveStatus::SetupNeeded { hint } => line(theme.destructive, "Setup needed", hint),
    }
}

/// The transcription row: what it shows, and its button.
struct Transcribing {
    title: String,
    detail: String,
    action: Option<(&'static str, ClientMessage)>,
}

fn transcribing(status: &TranscriptionStatus, resolve_here: bool) -> Option<Transcribing> {
    match status {
        // Downloaded by itself when the app starts; this is the way to try again after that failed.
        TranscriptionStatus::Idle { ready } => {
            (resolve_here && *ready == Some(false)).then(|| Transcribing {
                title: "Transcription isn't set up".into(),
                detail: "A one-time download of up to 1.6 GB".into(),
                action: Some(("Set up", ClientMessage::TranscriptionSetup)),
            })
        }
        TranscriptionStatus::Preparing { step, percent } => Some(Transcribing {
            title: match percent {
                Some(percent) => format!("{step} · {percent}%"),
                None => step.clone(),
            },
            detail: "Only the first time".into(),
            action: Some(("Cancel", ClientMessage::TranscriptionCancel)),
        }),
        TranscriptionStatus::Transcribing { file, done, total } => Some(Transcribing {
            title: "Transcribing".into(),
            detail: if *total > 1 {
                format!("{file} ({} of {total})", done + 1)
            } else {
                file.clone()
            },
            action: None,
        }),
    }
}

/// A card with a ring: `rounded-lg px-2.5 py-2 shadow-[0_0_0_1px_var(--border)]`.
fn card(theme: &Theme) -> gpui::Div {
    div()
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.border)
        .px_2p5()
        .py_2()
}

fn title(text: impl Into<SharedString>) -> gpui::Div {
    div()
        .text_token(TextSize::Xs)
        .font_weight(FontWeight::MEDIUM)
        .child(text.into())
}

fn detail(text: impl Into<SharedString>, theme: &Theme) -> gpui::Div {
    div()
        .mt_0p5()
        .text_token(TextSize::X2s)
        .text_color(theme.muted_foreground)
        .child(text.into())
}

fn link(
    id: &'static str,
    text: impl Into<SharedString>,
    theme: &Theme,
) -> gpui::Stateful<gpui::Div> {
    div()
        .id(id)
        .text_token(TextSize::X2s)
        .text_color(theme.primary)
        .cursor_pointer()
        .hover(|style| style.underline())
        .child(text.into())
}

impl Sidebar {
    /// AppUpdateCard.svelte: background checks that fail stay quiet; a failed download or install doesn't.
    fn app_update_card(
        &self,
        update: &AppUpdate,
        computer: &str,
        remote_page: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let state = &update.state;
        let visible = match state.status {
            DesktopUpdateStatus::Idle => false,
            DesktopUpdateStatus::Checking | DesktopUpdateStatus::UpToDate => self.asked,
            DesktopUpdateStatus::Error => self.asked || state.available_version.is_some(),
            _ => true,
        };
        if !visible {
            return None;
        }
        let version = release_name(
            state
                .downloaded_version
                .as_deref()
                .or(state.available_version.as_deref())
                .unwrap_or_default(),
        );
        let failed_before =
            state.not_applied.is_some() && state.not_applied == state.downloaded_version;
        let notes = (!state.notes.is_empty()).then(|| {
            div()
                .mt_1()
                .text_token(TextSize::X2s)
                .text_color(theme.muted_foreground)
                .child("What's new")
                .children(
                    state
                        .notes
                        .iter()
                        .map(|note| div().pl_3p5().child(format!("• {note}"))),
                )
        });
        let send = |message: ClientMessage| {
            cx.listener(move |this, _, _, cx| this.send(message.clone(), cx))
        };
        let body: Vec<AnyElement> = match state.status {
            DesktopUpdateStatus::Checking => vec![title("Looking for updates…").into_any_element()],
            DesktopUpdateStatus::UpToDate => vec![
                title("Linker-Edit is up to date").into_any_element(),
                detail(
                    format!("{} is the newest.", release_name(&state.current_version)),
                    theme,
                )
                .into_any_element(),
            ],
            DesktopUpdateStatus::Error => vec![
                title("Couldn't update").into_any_element(),
                detail(state.error.clone().unwrap_or_default(), theme).into_any_element(),
                link("update-try-again", "Try again", theme)
                    .mt_0p5()
                    .on_click(cx.listener(|this, _, _, cx| this.check_for_updates(cx)))
                    .into_any_element(),
            ],
            DesktopUpdateStatus::Available if state.download_url.is_some() => {
                let url = state.download_url.clone().unwrap_or_default();
                vec![
                    title(format!("Linker-Edit {version} is out")).into_any_element(),
                    div().children(notes).into_any_element(),
                    link("update-download", "Download it", theme)
                        .mt_0p5()
                        .on_click(move |_, _, cx| cx.open_url(&url))
                        .into_any_element(),
                ]
            }
            DesktopUpdateStatus::Available | DesktopUpdateStatus::Downloading => {
                let percent = state.percent;
                vec![
                    title(format!(
                        "Downloading Linker-Edit {version}{}",
                        percent.map_or("…".to_string(), |p| format!(" · {p}%"))
                    ))
                    .into_any_element(),
                    div()
                        .mt_1p5()
                        .h_1()
                        .overflow_hidden()
                        .rounded_full()
                        .bg(theme.border)
                        .child(
                            div()
                                .h_full()
                                .rounded_full()
                                .bg(theme.primary)
                                .w(gpui::relative((percent.unwrap_or(0.) / 100.) as f32)),
                        )
                        .into_any_element(),
                ]
                .into_iter()
                .chain(state.error.clone().map(|error| {
                    detail(
                        format!("The download stopped: {error} It tries again later."),
                        theme,
                    )
                    .mt_1()
                    .into_any_element()
                }))
                .collect()
            }
            DesktopUpdateStatus::Downloaded if update.waiting => vec![
                title("Updating when Resolve is free").into_any_element(),
                detail(
                    format!("Linker-Edit restarts into {version} once the running turn, render or open is done."),
                    theme,
                )
                .into_any_element(),
                link("update-not-now", "Not now", theme)
                    .mt_0p5()
                    .on_click(send(ClientMessage::CancelAppUpdate))
                    .into_any_element(),
            ],
            DesktopUpdateStatus::Downloaded => {
                let restart = if remote_page {
                    format!("Restart {computer} to update")
                } else {
                    "Restart to update".into()
                };
                let mut body = vec![if failed_before {
                    div()
                        .child(title(format!("Linker-Edit {version} didn't install")))
                        .child(detail(
                            format!(
                                "The last restart came back on {}.",
                                release_name(&state.current_version)
                            ),
                            theme,
                        ))
                        .into_any_element()
                } else {
                    title(format!("Linker-Edit {version} is ready")).into_any_element()
                }];
                if let Some(error) = &state.error {
                    body.push(
                        detail(error.clone(), theme)
                            .text_color(theme.destructive)
                            .into_any_element(),
                    );
                }
                body.push(div().children(notes).into_any_element());
                body.push(
                    div()
                        .mt_0p5()
                        .flex()
                        .items_center()
                        .gap_3()
                        .child(
                            link(
                                "update-install",
                                if failed_before || state.error.is_some() {
                                    "Try again".to_string()
                                } else {
                                    restart
                                },
                                theme,
                            )
                            .on_click(send(ClientMessage::InstallAppUpdate)),
                        )
                        .when(failed_before, |this| {
                            this.child(
                                link("update-download", "Download it", theme)
                                    .on_click(|_, _, cx| cx.open_url(RELEASES_URL)),
                            )
                        })
                        .into_any_element(),
                );
                body
            }
            DesktopUpdateStatus::Idle => return None,
        };
        Some(
            card(theme)
                .id("app-update")
                .children(body)
                .test_support()
                .into_any_element(),
        )
    }

    /// LinkerAccount.svelte.
    fn linker_account(
        &self,
        linker: &LinkerAccount,
        computer: &str,
        away: bool,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        // Linker sends the browser back to 127.0.0.1, so signing in only works on the computer
        // Linker-Edit runs on. A link, not a script: it opens in the system browser.
        let server = self
            .state
            .read(cx)
            .server
            .clone()
            .map(|server| origin(&server));
        let sign_in = server.map(|server| format!("{server}/api/linker/sign-in"));
        let (hover, foreground) = (theme.sidebar_row_hover, theme.sidebar_foreground);
        match linker {
            LinkerAccount::SignedIn {
                name,
                email,
                picture,
                plan,
                app_url,
            } => {
                let hovered = self.hovered.as_deref() == Some("linker-account");
                let app_url = app_url.clone();
                let initial = name.chars().next().map(|c| c.to_uppercase().to_string());
                div()
                    .id("linker-account")
                    .flex()
                    .items_center()
                    .gap_2p5()
                    .rounded(theme.radius.md)
                    .py_1p5()
                    .pr_1()
                    .pl_2p5()
                    .when(hovered, |this| this.bg(hover))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        this.set_hovered("linker-account", *hovered, cx)
                    }))
                    .child(match picture {
                        Some(picture) => img(picture.clone())
                            .size_5()
                            .flex_shrink_0()
                            .rounded_full()
                            .into_any_element(),
                        None => div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .justify_center()
                            .size_5()
                            .rounded_full()
                            .bg(theme.primary)
                            .text_token(TextSize::X2s)
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(theme.primary_foreground)
                            .child(initial.unwrap_or_default())
                            .into_any_element(),
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(title(name.clone()).truncate())
                            .child(
                                div()
                                    .truncate()
                                    .text_token(TextSize::X2s)
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "{}{}",
                                        plan.as_ref().map_or("Linker".into(), |plan| format!(
                                            "Linker {plan}"
                                        )),
                                        email
                                            .as_ref()
                                            .map_or(String::new(), |email| format!(" · {email}"))
                                    )),
                            ),
                    )
                    .when(hovered, |this| {
                        let accent = theme.accent;
                        this.child(
                            div()
                                .id("linker-account-menu")
                                .flex()
                                .items_center()
                                .justify_center()
                                .size_6()
                                .rounded(theme.radius.md)
                                .text_color(theme.muted_foreground)
                                .cursor_pointer()
                                .hover(move |style| style.bg(accent))
                                .on_click(cx.listener(
                                    move |this, event: &gpui::ClickEvent, window, cx| {
                                        let at = event.position();
                                        let items = vec![
                                            MenuItem::new(
                                                "open-linker",
                                                "Open Linker",
                                                SidebarAction::OpenUrl(app_url.clone()),
                                            )
                                            .icon(IconName::ExternalLink),
                                            MenuItem::new(
                                                "sign-out",
                                                "Sign out",
                                                SidebarAction::SignOut,
                                            )
                                            .icon(IconName::LogOut)
                                            .destructive(),
                                        ];
                                        this.open_menu(
                                            items,
                                            point(at.x + px(8.), at.y + px(12.)),
                                            true,
                                            window,
                                            cx,
                                        );
                                    },
                                ))
                                .child(Icon::new(IconName::Ellipsis).size(px(14.)))
                                .test_support(),
                        )
                    })
                    .test_support()
                    .into_any_element()
            }
            LinkerAccount::SigningIn => card(theme)
                .id("linker-signing-in")
                .child(title("Finish signing in in your browser"))
                .child(
                    div()
                        .mt_0p5()
                        .flex()
                        .gap_3()
                        .text_token(TextSize::X2s)
                        .when_some(sign_in, |this, url| {
                            this.child(
                                link("linker-sign-in-again", "Open it again", theme)
                                    .on_click(move |_, _, cx| cx.open_url(&url)),
                            )
                        })
                        .child(
                            div()
                                .id("linker-cancel")
                                .text_color(theme.muted_foreground)
                                .cursor_pointer()
                                .hover(move |style| style.text_color(foreground))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.send(ClientMessage::LinkerCancelSignIn, cx)
                                }))
                                .child("Cancel")
                                .test_support(),
                        ),
                )
                .test_support()
                .into_any_element(),
            LinkerAccount::SignedOut { problem } => {
                let row = match sign_in.filter(|_| !away) {
                    Some(url) => div()
                        .id("linker-sign-in")
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .rounded(theme.radius.md)
                        .px_2p5()
                        .py_1p5()
                        .text_color(theme.sidebar_muted_foreground)
                        .cursor_pointer()
                        .hover(move |style| style.bg(hover).text_color(foreground))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Optional. Brings your Linker account, and later its projects, to Linker-Edit.",
                            )
                            .build(window, cx)
                        })
                        .on_click(move |_, _, cx| cx.open_url(&url))
                        .child(Icon::new(IconName::LogIn).size(px(14.)))
                        .child(div().text_token(TextSize::Xs).child("Sign in with Linker"))
                        .test_support()
                        .into_any_element(),
                    None => div()
                        .flex()
                        .items_center()
                        .gap_2p5()
                        .px_2p5()
                        .py_1p5()
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .child(Icon::new(IconName::LogIn).size(px(14.)))
                        .child(format!("Sign in with Linker on {computer}"))
                        .into_any_element(),
                };
                div()
                    .child(row)
                    .when_some(problem.clone(), |this, problem| {
                        this.child(
                            div()
                                .px_2p5()
                                .pb_1()
                                .text_token(TextSize::X2s)
                                .text_color(theme.destructive)
                                .child(problem),
                        )
                    })
                    .into_any_element()
            }
        }
    }

    pub(super) fn render_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = Theme::of(cx).clone();
        let state = self.state.read(cx);
        let Some(app) = state.app.as_ref() else {
            return div().into_any_element();
        };
        let meta = app.meta.clone();
        let away = state.away();
        let status = resolve_status_line(&meta.resolve, &theme);
        // A computer that only drives an edit rig: nothing to set up here, so nothing to show.
        let control_only =
            !meta.resolve_here && matches!(meta.resolve, ResolveStatus::SetupNeeded { .. });

        let transcription = transcribing(&meta.transcription, meta.resolve_here).map(|row| {
            div()
                .id("transcription")
                .flex()
                .items_start()
                .gap_2p5()
                .px_2p5()
                .py_2()
                .child(
                    div().mt_0p5().flex_shrink_0().child(
                        Icon::new(IconName::AudioLines)
                            .size(px(14.))
                            .text_color(theme.primary),
                    ),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .child(title(row.title).truncate())
                        .child(detail(row.detail, &theme).truncate()),
                )
                .when_some(row.action, |this, (label, message)| {
                    this.child(
                        link("transcription-action", label, &theme)
                            .flex_shrink_0()
                            .on_click(
                                cx.listener(move |this, _, _, cx| this.send(message.clone(), cx)),
                            ),
                    )
                })
                .test_support()
        });

        let update = match (&meta.app_update, &meta.update) {
            (Some(update), _) => self.app_update_card(update, &meta.computer, away, &theme, cx),
            (None, Some(version)) => Some(
                card(&theme)
                    .id("npm-update")
                    .child(title(format!("Linker-Edit {version} is out")))
                    .child(
                        div()
                            .mt_0p5()
                            .flex()
                            .flex_wrap()
                            .gap_1()
                            .text_token(TextSize::X2s)
                            .text_color(theme.muted_foreground)
                            .child("Stop this one, then run")
                            .child(
                                div()
                                    .font_family(theme.mono_font_family)
                                    .child("npx linker-edit@latest"),
                            )
                            .child(div().ml(px(-4.)).child(".")),
                    )
                    .test_support()
                    .into_any_element(),
            ),
            (None, None) => None,
        };

        let account = self.linker_account(&meta.linker, &meta.computer, away, &theme, cx);

        let hover = theme.sidebar_row_hover;
        let settings = div()
            .id("settings")
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size_8()
            .rounded(theme.radius.control)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .tooltip(|window, cx| {
                gpui_kit::component::tooltip::Tooltip::new("Settings (Ctrl+,)").build(window, cx)
            })
            .on_click(cx.listener(|_, _, _, cx| cx.emit(SidebarEvent::Settings)))
            .child(
                Icon::new(IconName::Settings)
                    .size(px(16.))
                    .text_color(theme.sidebar_icon),
            )
            .test_support();

        let resolve_hovered = self.hovered.as_deref() == Some("resolve-status");
        let tip = if status.detail.is_empty() {
            format!("{}\nClick to refresh", status.title)
        } else {
            format!("{}\n{}\nClick to refresh", status.title, status.detail)
        };
        let resolve = (!control_only).then(|| {
            div()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .id("resolve-status")
                        .flex()
                        .items_start()
                        .gap_2()
                        .min_h_8()
                        .w_full()
                        .px_2()
                        .py_1p5()
                        .rounded(theme.radius.control)
                        .cursor_pointer()
                        .when(resolve_hovered, |this| this.bg(hover))
                        .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                            this.set_hovered("resolve-status", *hovered, cx)
                        }))
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(tip.clone())
                                .build(window, cx)
                        })
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.send(ClientMessage::RefreshResolve, cx)
                        }))
                        .child(
                            div()
                                .mt_1p5()
                                .size_2()
                                .flex_shrink_0()
                                .rounded_full()
                                .bg(status.tone),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .child(title(status.title.clone()).truncate())
                                .when(!status.detail.is_empty(), |this| {
                                    this.child(detail(status.detail.clone(), &theme))
                                }),
                        )
                        .child(
                            div().mt_0p5().flex_shrink_0().child(
                                Icon::new(IconName::RefreshCw)
                                    .size(px(14.))
                                    .text_color(theme.muted_foreground)
                                    .when(!resolve_hovered, |this| this.opacity(0.)),
                            ),
                        )
                        .test_support(),
                )
                .when(
                    matches!(meta.resolve, ResolveStatus::SetupNeeded { .. }),
                    |this| {
                        this.child(
                            link("resolve-not-here", "Resolve won't be on this computer", &theme)
                                .mb_1()
                                .ml_6()
                                .tooltip(|window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(
                                        "For a computer that only controls your edit computer. Comes back if Resolve is installed here.",
                                    )
                                    .build(window, cx)
                                })
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.send(
                                        ClientMessage::SetResolveHere {
                                            resolve_here: false,
                                        },
                                        cx,
                                    )
                                })),
                        )
                    },
                )
        });

        div()
            .id("sidebar-footer")
            .flex()
            .flex_col()
            .gap_1()
            .px(INSET)
            .pt_1()
            .pb(INSET)
            .children(transcription)
            .children(update)
            .child(account)
            .child(
                div()
                    .flex()
                    .items_start()
                    .gap_1()
                    .child(settings)
                    .children(resolve),
            )
            .test_support()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn says_resolves_state_in_one_line() {
        let theme = Theme::light();
        let connected = ResolveStatus::Connected {
            product: "DaVinci Resolve Studio".into(),
            version: "20.2.1.6".into(),
            project: Some("Promo".into()),
            project_id: None,
            database: None,
            timeline: Some("AI - Cut".into()),
        };
        let line = resolve_status_line(&connected, &theme);
        assert_eq!(line.title, "DaVinci Resolve Studio 20.2.1");
        assert_eq!(line.detail, "Promo · AI - Cut");
        assert_eq!(line.tone, theme.success);
        let line = resolve_status_line(&ResolveStatus::NotRunning, &theme);
        assert_eq!(line.detail, "Start Resolve, then refresh.");
    }

    #[test]
    fn offers_the_speech_engine_only_where_resolve_is() {
        let idle = TranscriptionStatus::Idle { ready: Some(false) };
        assert!(transcribing(&idle, true).is_some());
        assert!(transcribing(&idle, false).is_none());
        let busy = TranscriptionStatus::Transcribing {
            file: "a.wav".into(),
            done: 0,
            total: 3,
        };
        assert_eq!(transcribing(&busy, false).unwrap().detail, "a.wav (1 of 3)");
    }
}
