//! AppUpdateCard.svelte: the desktop app's own updates (AppMeta.appUpdate, server/appUpdate.ts)
//! as a card in the sidebar's footer. On another of the user's computers it is that computer's
//! app: restarting restarts Linker-Edit there. It only shows the state and sends
//! check_app_update / install_app_update / cancel_app_update; the server and the updater do
//! the rest.
//!
//! The sidebar footer (SidebarFooter.svelte) places it; `asked` is the footer's "the user clicked
//! Check for updates", so a check says how it went even when there's nothing new.

use gpui::{
    App, ClickEvent, FontWeight, InteractiveElement, IntoElement, ParentElement, RenderOnce,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use linker_edit_protocol::release_name::{RELEASES_URL, release_name};
use linker_edit_protocol::{AppUpdate, DesktopUpdateStatus};

use crate::components::OnClick;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::settings_parts::{progress_bar, ring};

#[derive(IntoElement)]
pub struct AppUpdateCard {
    update: AppUpdate,
    computer: String,
    /// The window is on another computer's Linker-Edit.
    remote_page: bool,
    asked: bool,
    on_install: OnClick,
    on_cancel: OnClick,
    on_check: OnClick,
}

type Handler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

impl AppUpdateCard {
    pub fn new(
        update: AppUpdate,
        computer: impl Into<String>,
        remote_page: bool,
        asked: bool,
    ) -> Self {
        let none: fn() -> Handler = || Box::new(|_, _, _| {});
        AppUpdateCard {
            update,
            computer: computer.into(),
            remote_page,
            asked,
            on_install: none(),
            on_cancel: none(),
            on_check: none(),
        }
    }

    pub fn on_install(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_install = Box::new(f);
        self
    }

    pub fn on_cancel(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_cancel = Box::new(f);
        self
    }

    pub fn on_check(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_check = Box::new(f);
        self
    }
}

/// Whether the card shows: background checks that fail, say offline, stay quiet; a failed
/// download or install doesn't.
pub fn update_card_visible(update: &AppUpdate, asked: bool) -> bool {
    match update.state.status {
        DesktopUpdateStatus::Idle => false,
        DesktopUpdateStatus::Checking | DesktopUpdateStatus::UpToDate => asked,
        DesktopUpdateStatus::Error => asked || update.state.available_version.is_some(),
        _ => true,
    }
}

/// `text-2xs text-primary hover:underline`.
fn link(id: &'static str, label: impl Into<SharedString>, cx: &App) -> gpui::Stateful<gpui::Div> {
    let theme = Theme::of(cx);
    div()
        .id(id)
        .text_token(TextSize::X2s)
        .text_color(theme.primary)
        .cursor_pointer()
        .hover(|style| style.underline())
        .child(label.into())
}

impl RenderOnce for AppUpdateCard {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        if !update_card_visible(&self.update, self.asked) {
            return div().into_any_element();
        }
        let state = &self.update.state;
        let version = release_name(
            state
                .downloaded_version
                .as_deref()
                .or(state.available_version.as_deref())
                .unwrap_or(""),
        );
        let failed_before =
            state.not_applied.is_some() && state.not_applied == state.downloaded_version;
        let restart_label = if self.remote_page {
            format!("Restart {} to update", self.computer)
        } else {
            "Restart to update".into()
        };
        let title = |text: String| {
            div()
                .text_token(TextSize::Xs)
                .font_weight(FontWeight::MEDIUM)
                .child(text)
        };
        let line = |text: String| {
            div()
                .mt_0p5()
                .text_token(TextSize::X2s)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        // <details> "What's new": closed until clicked.
        let notes_open = window.use_keyed_state("update-notes-open", cx, |_, _| false);
        let notes = (!state.notes.is_empty()).then(|| {
            let open = *notes_open.read(cx);
            let toggle = notes_open.clone();
            let foreground = theme.foreground;
            div()
                .mt_1()
                .text_token(TextSize::X2s)
                .text_color(theme.muted_foreground)
                .child(
                    div()
                        .id("update-notes")
                        .cursor_pointer()
                        .hover(move |style| style.text_color(foreground))
                        .on_click(move |_, _, cx| {
                            toggle.update(cx, |open, cx| {
                                *open = !*open;
                                cx.notify();
                            })
                        })
                        .child(format!("{} What's new", if open { "▾" } else { "▸" }))
                        .test_support(),
                )
                .when(open, |this| {
                    this.child(
                        div()
                            .mt_1()
                            .pl(px(14.))
                            .flex()
                            .flex_col()
                            .gap_0p5()
                            .children(state.notes.iter().map(|note| format!("• {note}"))),
                    )
                })
        });

        let body = div().flex().flex_col();
        let body = match state.status {
            DesktopUpdateStatus::Checking => body.child(title("Looking for updates…".into())),
            DesktopUpdateStatus::UpToDate => body
                .child(title("Linker-Edit is up to date".into()))
                .child(line(format!(
                    "{} is the newest.",
                    release_name(&state.current_version)
                ))),
            DesktopUpdateStatus::Error => body
                .child(title("Couldn't update".into()))
                .child(line(state.error.clone().unwrap_or_default()).line_clamp(3))
                .child(
                    link("update-try-again", "Try again", cx)
                        .mt_0p5()
                        .on_click(self.on_check)
                        .test_support(),
                ),
            DesktopUpdateStatus::Available if state.download_url.is_some() => {
                let url = state.download_url.clone().unwrap_or_default();
                body.child(title(format!("Linker-Edit {version} is out")))
                    .children(notes)
                    .child(
                        link("update-download", "Download it", cx)
                            .mt_0p5()
                            .on_click(move |_, _, cx| cx.open_url(&url))
                            .test_support(),
                    )
            }
            DesktopUpdateStatus::Available | DesktopUpdateStatus::Downloading => body
                .child(title(format!(
                    "Downloading Linker-Edit {version}{}",
                    state
                        .percent
                        .map_or("…".into(), |percent| format!(" · {percent}%"))
                )))
                .child(div().mt_1p5().child(progress_bar(Some(state.percent.unwrap_or(0.)), &theme)))
                .when_some(state.error.clone(), |this, error| {
                    this.child(
                        line(format!("The download stopped: {error} It tries again later."))
                            .mt_1()
                            .line_clamp(2),
                    )
                }),
            DesktopUpdateStatus::Downloaded if self.update.waiting => body
                .child(title("Updating when Resolve is free".into()))
                .child(line(format!(
                    "Linker-Edit restarts into {version} once the running turn, render or open is done."
                )))
                .child(
                    link("update-not-now", "Not now", cx)
                        .mt_0p5()
                        .on_click(self.on_cancel)
                        .test_support(),
                ),
            DesktopUpdateStatus::Downloaded => {
                let header = if failed_before {
                    div()
                        .child(title(format!("Linker-Edit {version} didn't install")))
                        .child(line(format!(
                            "The last restart came back on {}.",
                            release_name(&state.current_version)
                        )))
                } else {
                    div().child(title(format!("Linker-Edit {version} is ready")))
                };
                body.child(header)
                    .when_some(state.error.clone(), |this, error| {
                        this.child(
                            line(error)
                                .text_color(theme.destructive)
                                .line_clamp(3),
                        )
                    })
                    .children(notes)
                    .child(
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
                                        restart_label
                                    },
                                    cx,
                                )
                                .on_click(self.on_install)
                                .test_support(),
                            )
                            .when(failed_before, |this| {
                                this.child(
                                    link("update-releases", "Download it", cx)
                                        .on_click(|_, _, cx| cx.open_url(RELEASES_URL))
                                        .test_support(),
                                )
                            }),
                    )
            }
            DesktopUpdateStatus::Idle => body,
        };
        div()
            .id("app-update-card")
            .mx_2()
            .rounded(theme.radius.lg)
            .px_2p5()
            .py_2()
            .shadow(vec![ring(theme.border, 1.)])
            .child(body)
            .test_support()
            .into_any_element()
    }
}
