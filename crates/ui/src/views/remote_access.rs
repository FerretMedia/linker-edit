//! RemoteAccessSettings.svelte: letting the user's other computers in over Tailscale Serve. The
//! settings page asks Tailscale again whenever this section opens (its `onMount`).

use gpui::{
    App, ClickEvent, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder,
};
use gpui_kit::TestSupportExt;
use linker_edit_protocol::{ClientMessage, RemoteAccess};

use crate::components::OnClick;
use crate::icons::IconName;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::settings_parts::{
    confirm_then, emphasized, icon_button, linked_text, outline_button, primary_button,
};

#[derive(IntoElement)]
pub struct RemoteAccessSettings {
    state: Entity<AppState>,
    copied: bool,
    on_copy: OnClick,
}

impl RemoteAccessSettings {
    pub fn new(
        state: Entity<AppState>,
        copied: bool,
        on_copy: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        RemoteAccessSettings {
            state,
            copied,
            on_copy: Box::new(on_copy),
        }
    }
}

fn send(
    state: &Entity<AppState>,
    message: ClientMessage,
) -> impl Fn(&ClickEvent, &mut Window, &mut App) + 'static {
    let state = state.clone();
    move |_, _, cx| state.read(cx).send(message.clone())
}

impl RenderOnce for RemoteAccessSettings {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let app_state = self.state.read(cx);
        let Some(app) = app_state.app.as_ref() else {
            return div().into_any_element();
        };
        let remote = app.meta.remote.clone();
        // Turning it off from a remote device cuts that device off.
        let via_tailscale = !app_state.on_this_computer();
        let body = div().id("remote-access").flex().flex_col().gap_4();

        let body = match &remote {
            RemoteAccess::Checking => body.child(
                div()
                    .text_token(TextSize::Sm)
                    .text_color(theme.muted_foreground)
                    .child("Checking Tailscale…"),
            ),
            RemoteAccess::Unavailable { hint } => body
                .child(div().text_token(TextSize::Sm).child(linked_text(
                    "remote-hint",
                    hint,
                    &theme,
                )))
                .child(
                    div().flex().child(
                        outline_button("remote-check-again", &theme)
                            .px_2p5()
                            .text_token(TextSize::Sm)
                            .on_click(send(&self.state, ClientMessage::RefreshRemoteAccess))
                            .child("Check again")
                            .test_support(),
                    ),
                ),
            RemoteAccess::Off { url, .. }
            | RemoteAccess::Switching { url }
            | RemoteAccess::On { url, .. } => {
                let on = matches!(remote, RemoteAccess::On { .. });
                let address = div()
                    .flex()
                    .flex_col()
                    .gap_1()
                    .text_token(TextSize::Xs)
                    .child(div().text_color(theme.muted_foreground).child(if on {
                        "On your other computer, pick this one in Linker-Edit's computer menu, or open"
                    } else {
                        "Address once it's on"
                    }))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .id("remote-url")
                                    .flex_1()
                                    .min_w_0()
                                    .font_family(theme.mono_font_family)
                                    .text_token(TextSize::Xs)
                                    .when(!on, |this| this.text_color(theme.muted_foreground))
                                    .child(url.clone())
                                    .test_support(),
                            )
                            .when(on, |this| {
                                this.child(
                                    icon_button(
                                        "remote-copy",
                                        if self.copied {
                                            IconName::Check
                                        } else {
                                            IconName::Copy
                                        },
                                        &theme,
                                    )
                                    .tooltip(|window, cx| {
                                        gpui_kit::component::tooltip::Tooltip::new("Copy address")
                                            .build(window, cx)
                                    })
                                    .on_click(self.on_copy)
                                    .test_support(),
                                )
                            }),
                    );
                let note = match &remote {
                    RemoteAccess::On { owner, .. } => Some(
                        div()
                            .text_token(TextSize::Xs)
                            .text_color(theme.muted_foreground)
                            .child(emphasized(
                                "For devices logged in as ",
                                owner,
                                ". It stays on after Linker-Edit restarts, until you turn it off.",
                                theme.foreground,
                            ))
                            .into_any_element(),
                    ),
                    RemoteAccess::Off {
                        problem: Some(problem),
                        ..
                    } => Some(
                        div()
                            .text_token(TextSize::Sm)
                            .text_color(theme.destructive)
                            .child(linked_text("remote-problem", problem, &theme))
                            .into_any_element(),
                    ),
                    _ => None,
                };
                let (dot, label) = match remote {
                    RemoteAccess::On { .. } => (theme.success, "On"),
                    RemoteAccess::Switching { .. } => (theme.warning, "Switching…"),
                    _ => (theme.muted_foreground, "Off"),
                };
                let switching = matches!(remote, RemoteAccess::Switching { .. });
                let state = self.state.clone();
                let toggle = if on {
                    outline_button("remote-turn-off", &theme)
                        .ml_auto()
                        .text_token(TextSize::Sm)
                        .on_click(move |_, window, cx| {
                            let state = state.clone();
                            let off = move |cx: &mut App| {
                                state
                                    .read(cx)
                                    .send(ClientMessage::SetRemoteAccess { enabled: false })
                            };
                            if via_tailscale {
                                confirm_then(
                                    "You're connected through Tailscale. Turning remote access off disconnects this device.",
                                    window,
                                    cx,
                                    off,
                                );
                            } else {
                                off(cx);
                            }
                        })
                        .child("Turn off")
                } else {
                    primary_button("remote-turn-on", &theme)
                        .ml_auto()
                        .text_token(TextSize::Sm)
                        .when(switching, |this| this.opacity(0.5).cursor_default())
                        .when(!switching, |this| {
                            this.on_click(send(
                                &self.state,
                                ClientMessage::SetRemoteAccess { enabled: true },
                            ))
                        })
                        .child("Turn on")
                };
                body.child(address).children(note).child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .border_t_1()
                        .border_color(theme.border)
                        .pt_4()
                        .child(
                            div()
                                .id("remote-state")
                                .flex()
                                .items_center()
                                .gap_2()
                                .text_token(TextSize::Sm)
                                .child(div().size_2().rounded_full().bg(dot))
                                .child(label)
                                .test_support(),
                        )
                        .child(toggle.test_support()),
                )
            }
        };
        body.font_weight(FontWeight::NORMAL)
            .test_support()
            .into_any_element()
    }
}
