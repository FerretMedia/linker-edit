//! LinkerConnectSettings.svelte: Linker Connect, like T3 Connect. This computer gets its own
//! address at Linker, and the user's other computers signed in to the same Linker account open
//! it from anywhere. The linked computers (unlink one), the devices signed in to this one (sign
//! one out), and turning it on and off. The settings page asks Linker again whenever this
//! section opens (its `onMount`).

use gpui::{
    AnyElement, App, ClickEvent, Entity, InteractiveElement, IntoElement, ParentElement,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::{ClientMessage, ConnectDeviceKind, LinkerConnect, LinkerConnectState};

use crate::components::OnClick;
use crate::icons::IconName;
use crate::state::{AppState, origin};
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::settings_parts::{
    SettingsRow, Switch, confirm_then, group, medium_date_time, outline_button,
};

#[derive(IntoElement)]
pub struct LinkerConnectSettings {
    state: Entity<AppState>,
    copied: bool,
    on_copy: OnClick,
    on_sign_in: OnClick,
}

impl LinkerConnectSettings {
    pub fn new(
        state: Entity<AppState>,
        copied: bool,
        on_copy: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
        on_sign_in: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        LinkerConnectSettings {
            state,
            copied,
            on_copy: Box::new(on_copy),
            on_sign_in: Box::new(on_sign_in),
        }
    }
}

fn when(iso: Option<&str>) -> String {
    iso.map_or_else(|| "never".into(), medium_date_time)
}

/// Turning it on or off; off through this computer's own tunnel asks first, since it cuts this
/// window off.
fn set_enabled(
    state: Entity<AppState>,
    enabled: bool,
    through_tunnel: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let send = move |cx: &mut App| state.read(cx).send(ClientMessage::SetConnect { enabled });
    if !enabled && through_tunnel {
        confirm_then(
            "You're connected through Linker Connect. Turning it off disconnects this window.",
            window,
            cx,
            send,
        );
    } else {
        send(cx);
    }
}

impl RenderOnce for LinkerConnectSettings {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let app_state = self.state.read(cx);
        let Some(app) = app_state.app.as_ref() else {
            return div().into_any_element();
        };
        let connect: LinkerConnect = app.meta.connect.clone();
        let computer = app.meta.computer.clone();
        // Opened through this computer's own tunnel, turning it off or unlinking it cuts this
        // window off.
        let through_tunnel = match (&connect.url, &app_state.server) {
            (Some(url), Some(server)) => origin(url) == server.origin(),
            _ => false,
        };
        let state = self.state.clone();

        if connect.state == LinkerConnectState::SignedOut {
            return div()
                .id("linker-connect")
                .flex()
                .flex_col()
                .child(group(
                    "Linker Connect",
                    Some("Open this computer from your other computers, from anywhere. It needs your Linker account.".into()),
                    [SettingsRow::new("Sign in with Linker first")
                        .description("Then sign in with the same account on the computers you work from.")
                        .control(
                            outline_button("connect-sign-in", &theme)
                                .on_click(self.on_sign_in)
                                .child("Linker account")
                                .test_support(),
                        )
                        .into_any_element()],
                    &theme,
                ))
                .test_support()
                .into_any_element();
        }

        let description = match connect.state {
            LinkerConnectState::Starting => connect.step.clone().unwrap_or("Starting…".into()),
            LinkerConnectState::On if connect.connected => {
                "On. Reachable while Linker-Edit runs here.".into()
            }
            LinkerConnectState::On => "Connecting to Cloudflare…".into(),
            _ => format!(
                "Gives {computer} its own address at Linker. One account links up to {} computers.",
                connect.limit
            ),
        };
        let dot = match connect.state {
            LinkerConnectState::On if connect.connected => theme.success,
            LinkerConnectState::Off => theme.muted_foreground,
            _ => theme.warning,
        };
        let starting = connect.state == LinkerConnectState::Starting;
        let checked = matches!(
            connect.state,
            LinkerConnectState::On | LinkerConnectState::Starting
        );
        let toggle_state = state.clone();
        let mut main_row = SettingsRow::new(format!("Linker Connect on {computer}"))
            .id("connect-row")
            .description(description)
            .control(div().size_2().rounded_full().bg(dot))
            .control(Switch::new(
                "connect-switch",
                checked,
                move |checked, window, cx| {
                    if !starting {
                        set_enabled(toggle_state.clone(), checked, through_tunnel, window, cx);
                    }
                },
            ));
        if let Some(problem) = &connect.problem {
            main_row = main_row.detail(
                div()
                    .mt_1()
                    .text_token(TextSize::Xs)
                    .text_color(theme.destructive)
                    .child(problem.clone()),
            );
        }
        let mut rows = vec![main_row.into_any_element()];
        if let (Some(url), LinkerConnectState::On) = (&connect.url, connect.state) {
            let copied = self.copied;
            rows.push(
                SettingsRow::new("Address")
                    .description(format!(
                        "On your other computer, pick {computer} in Linker-Edit's computer menu."
                    ))
                    .detail(
                        div()
                            .mt_1()
                            .font_family(theme.mono_font_family)
                            .text_token(TextSize::Xs)
                            .child(url.clone()),
                    )
                    .control(
                        outline_button("connect-copy", &theme)
                            .on_click(self.on_copy)
                            .child(
                                Icon::new(if copied {
                                    IconName::Check
                                } else {
                                    IconName::Copy
                                })
                                .size(px(14.)),
                            )
                            .child("Copy")
                            .test_support(),
                    )
                    .into_any_element(),
            );
        }

        let count = connect.computers.as_ref().map_or(0, Vec::len);
        let linked_here = connect.computers.as_ref().is_some_and(|computers| {
            computers
                .iter()
                .any(|entry| entry.id == connect.computer_id)
        });
        let mut computers: Vec<AnyElement> = match &connect.computers {
            None => vec![SettingsRow::new("Looking them up…").into_any_element()],
            Some(list) if list.is_empty() => vec![
                SettingsRow::new("None yet")
                    .description(format!("Turn Linker Connect on to link {computer}."))
                    .into_any_element(),
            ],
            Some(list) => list
                .iter()
                .map(|entry| {
                    let here = entry.id == connect.computer_id;
                    let online = match entry.online {
                        Some(true) => "Online",
                        Some(false) => "Offline",
                        None => "Not known",
                    };
                    let (id, name) = (entry.id.clone(), entry.name.clone());
                    let state = state.clone();
                    SettingsRow::new(format!(
                        "{}{}",
                        entry.name,
                        if here { " (this computer)" } else { "" }
                    ))
                    .id(SharedString::from(format!("computer-{}", entry.id)))
                    .description(format!(
                        "{online} · linked {}",
                        when(Some(&entry.linked_at))
                    ))
                    .control(
                        Icon::new(IconName::Monitor)
                            .size(px(14.))
                            .text_color(theme.muted_foreground),
                    )
                    .control(
                        outline_button(SharedString::from(format!("unlink-{}", entry.id)), &theme)
                            .on_click(move |_, window, cx| {
                                let text = if here {
                                    format!("Unlink {name}? It loses its address at Linker, and every device signed in to it is signed out.")
                                } else {
                                    format!("Unlink {name}? Its tunnel stops working until you turn Linker Connect on there again.")
                                };
                                let text = if here && through_tunnel {
                                    format!("{text} This window is disconnected too.")
                                } else {
                                    text
                                };
                                let (state, computer_id) = (state.clone(), id.clone());
                                confirm_then(&text, window, cx, move |cx| {
                                    state.read(cx).send(ClientMessage::ConnectUnlink { computer_id })
                                });
                            })
                            .child("Unlink")
                            .test_support(),
                    )
                    .into_any_element()
                })
                .collect(),
        };
        if connect.at_limit && !linked_here {
            let state = state.clone();
            computers.push(
                SettingsRow::new(format!("Unlink one to link {computer}"))
                    .description(format!(
                        "Each Linker account links up to {} computers.",
                        connect.limit
                    ))
                    .control(
                        outline_button("connect-try-again", &theme)
                            .on_click(move |_, window, cx| {
                                set_enabled(state.clone(), true, through_tunnel, window, cx)
                            })
                            .child("Try again")
                            .test_support(),
                    )
                    .into_any_element(),
            );
        }

        let devices: Vec<AnyElement> = if connect.devices.is_empty() {
            vec![SettingsRow::new("None").into_any_element()]
        } else {
            connect
                .devices
                .iter()
                .map(|device| {
                    let (state, device_id) = (state.clone(), device.id.clone());
                    SettingsRow::new(device.name.clone())
                        .id(SharedString::from(format!("device-{}", device.id)))
                        .description(format!(
                            "{}Signed in {} · last used {}",
                            if device.kind == ConnectDeviceKind::Native {
                                "Native app · "
                            } else {
                                ""
                            },
                            when(Some(&device.created_at)),
                            when(Some(&device.last_seen_at))
                        ))
                        .control(
                            outline_button(
                                SharedString::from(format!("sign-out-device-{}", device.id)),
                                &theme,
                            )
                            .on_click(move |_, _, cx| {
                                state.read(cx).send(ClientMessage::ConnectSignOutDevice {
                                    device_id: device_id.clone(),
                                })
                            })
                            .child(Icon::new(IconName::X).size(px(14.)))
                            .child("Sign out")
                            .test_support(),
                        )
                        .into_any_element()
                })
                .collect()
        };

        div()
            .id("linker-connect")
            .flex()
            .flex_col()
            .gap_8()
            .child(group(
                "Linker Connect",
                Some("Open this computer from your other computers, from anywhere: they sign in with the same Linker account. Resolve and the agents keep running here.".into()),
                rows,
                &theme,
            ))
            .child(group(
                format!(
                    "Your linked computers{}",
                    if connect.computers.is_some() {
                        format!(" · {count} of {}", connect.limit)
                    } else {
                        String::new()
                    }
                ),
                Some("The edit computers linked to your Linker account. Computers you only work from don't count.".into()),
                computers,
                &theme,
            ))
            .child(group(
                "Signed-in devices",
                Some(format!("Computers and the iPad app that opened {computer} through Linker Connect. Signing one out cuts it off at once.").into()),
                devices,
                &theme,
            ))
            .test_support()
            .into_any_element()
    }
}
