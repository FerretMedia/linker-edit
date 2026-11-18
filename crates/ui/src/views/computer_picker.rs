//! ComputerPicker.svelte, with what desktop/main.ts did for it: the computer menu at the top of the
//! sidebar. It lists your computers (this computer's server's `GET /api/computers`: the ones
//! reached over Tailscale and those linked with Linker Connect) and opens one in the window.
//!
//! Opening another computer puts the window's link on that computer's server:
//! - over Tailscale, its plain `/ws` (Tailscale Serve lets in your own devices);
//! - over Linker Connect, as a native client (docs/sync-plan.md, "Native clients"): this
//!   computer's server asks it for a one-time code with your Linker sign-in
//!   (`POST /api/connect/open`), the window swaps the code for a session
//!   (`POST <computer>/api/connect/native`) and sends it as `Authorization: Bearer` on `/ws`.
//!
//! "This computer" goes back. As in main.ts, a computer that can't be reached, or doesn't let you
//! in, sends the window back home with a message saying so.

use std::time::Duration;

use gpui::{
    AppContext, Context, Entity, EventEmitter, FocusHandle, FontWeight, InteractiveElement,
    IntoElement, ParentElement, PromptLevel, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window, anchored, deferred, div, point, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use linker_edit_protocol::release_name::{is_newer, release_name};
use linker_edit_protocol::{ClientMessage, ComputerVia, Computers, RemoteAccess};

use crate::http;
use crate::icons::IconName;
use crate::state::{AppState, Auth, ServerAddress, origin};
use crate::theme::{TextSize, TextStyled, Theme};
use crate::transport;

/// main.ts's fetch timeouts: the list, and signing in to a Linker Connect computer.
const LIST_TIMEOUT: Duration = Duration::from_secs(15);
const SIGN_IN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long another computer gets to answer before the window goes back home.
const REACH_TIMEOUT: Duration = Duration::from_secs(15);

/// The user's computers as GET /api/computers lists them, with this one's address.
#[derive(Clone, Debug, PartialEq)]
pub struct ComputerList {
    pub computers: Computers,
    pub home: String,
}

/// One row of the menu.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub url: String,
    pub version: String,
    pub home: bool,
    pub via: Option<ComputerVia>,
    pub online: bool,
}

impl ComputerList {
    pub fn entries(&self) -> Vec<Entry> {
        let mut entries = vec![Entry {
            name: self.computers.name.clone(),
            url: self.home.clone(),
            version: self.computers.version.clone().unwrap_or_default(),
            home: true,
            via: None,
            online: true,
        }];
        entries.extend(self.computers.others.iter().map(|other| Entry {
            name: other.name.clone(),
            url: other.url.clone(),
            version: other.version.clone(),
            home: false,
            via: other.via,
            online: other.online != Some(false),
        }));
        entries
    }
}

/// What the menu asks of the rest of the window.
pub enum ComputerPickerEvent {
    /// "Remote access…": the settings page's remote access section.
    RemoteAccess,
}

/// How the window talks to servers over HTTP; a test answers instead.
pub trait Http: Send + Sync + 'static {
    fn computers(&self, home: &ServerAddress) -> Result<Computers, String>;
    /// A Linker Connect session on `url`, through this computer's server at `home`.
    fn connect_session(&self, home: &ServerAddress, url: &str) -> Result<String, String>;
}

/// The real thing: the window's HTTP client, blocking, on a background thread.
pub struct Reqwest;

#[derive(serde::Deserialize)]
struct Answer {
    url: Option<String>,
    session: Option<String>,
    error: Option<String>,
}

fn post(url: &str, body: serde_json::Value) -> Result<Answer, String> {
    let response = http::client(SIGN_IN_TIMEOUT)?
        .post(url)
        .json(&body)
        .send()
        .map_err(|error| error.to_string())?;
    let status = response.status();
    let answer: Answer = response.json().unwrap_or(Answer {
        url: None,
        session: None,
        error: None,
    });
    if !status.is_success() {
        return Err(answer
            .error
            .unwrap_or_else(|| format!("Linker-Edit answered {}.", status.as_u16())));
    }
    Ok(answer)
}

impl Http for Reqwest {
    fn computers(&self, home: &ServerAddress) -> Result<Computers, String> {
        http::get(home, "/api/computers", LIST_TIMEOUT)?
            .json()
            .map_err(|error| error.to_string())
    }

    fn connect_session(&self, home: &ServerAddress, url: &str) -> Result<String, String> {
        let computer = origin(url);
        let enter = post(
            &home.url("/api/connect/open"),
            serde_json::json!({ "url": computer }),
        )?
        .url
        .ok_or("Linker-Edit didn't say where to sign in.")?;
        let code = code_of(&enter).ok_or("Linker-Edit didn't give a sign-in code.")?;
        post(
            &format!("{computer}/api/connect/native"),
            serde_json::json!({ "code": code }),
        )?
        .session
        .ok_or_else(|| "That computer didn't give a session.".into())
    }
}

/// The one-time code in `/api/connect/enter?code=…`.
pub fn code_of(enter: &str) -> Option<String> {
    let query = enter.split_once('?')?.1;
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=')?;
        (key == "code").then(|| percent_decode(value))
    })
}

fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' if index + 2 < bytes.len() => {
                match u8::from_str_radix(&value[index + 1..index + 3], 16) {
                    Ok(byte) => {
                        out.push(byte);
                        index += 3;
                    }
                    Err(_) => {
                        out.push(b'%');
                        index += 1;
                    }
                }
            }
            b'+' => {
                out.push(b' ');
                index += 1;
            }
            byte => {
                out.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How the window reaches a server: a test makes in-process links instead.
pub type Connect = std::rc::Rc<dyn Fn(&ServerAddress) -> anyhow::Result<crate::state::Link>>;

pub struct ComputerPicker {
    state: Entity<AppState>,
    http: std::sync::Arc<dyn Http>,
    connect: Connect,
    open: bool,
    list: Option<ComputerList>,
    failed: bool,
    loading: Option<Task<()>>,
    focus: FocusHandle,
    /// The pointer is on the toggle, whose own click opens and closes the menu.
    on_toggle: bool,
    /// Watches a computer just opened: back home if it doesn't answer in time.
    reaching: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ComputerPickerEvent> for ComputerPicker {}

impl ComputerPicker {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let connect: Connect =
            std::rc::Rc::new(|server| transport::connect(&server.base, server.auth.clone()));
        Self::with(state, std::sync::Arc::new(Reqwest), connect, window, cx)
    }

    /// With `http` and `connect` of a test's.
    pub fn with(
        state: Entity<AppState>,
        http: std::sync::Arc<dyn Http>,
        connect: Connect,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![cx.observe_in(&state, window, |this, state, window, cx| {
            let state = state.read(cx);
            if state.away() && state.signed_out {
                let server = state.server.as_ref().map(|server| server.base.as_str());
                let linker = this.via(server) == Some(ComputerVia::Linker);
                this.go_home(
                    "That computer didn't let you in.",
                    if linker {
                        "Linker Connect only lets in computers signed in to the same Linker account. Pick it again in the computer menu."
                    } else {
                        "Remote access only lets in devices logged in to the same Tailscale account as that computer."
                    },
                    window,
                    cx,
                );
            } else if state.connected {
                this.reaching = None;
            }
            cx.notify();
        })];
        let mut picker = ComputerPicker {
            state,
            http,
            connect,
            open: false,
            list: None,
            failed: false,
            loading: None,
            focus: cx.focus_handle(),
            on_toggle: false,
            reaching: None,
            _subscriptions: subscriptions,
        };
        picker.load(cx);
        picker
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn list(&self) -> Option<&ComputerList> {
        self.list.as_ref()
    }

    fn via(&self, server: Option<&str>) -> Option<ComputerVia> {
        let server = origin(server?);
        self.list
            .as_ref()?
            .computers
            .others
            .iter()
            .find(|other| origin(&other.url) == server)
            .and_then(|other| other.via)
    }

    /// Asks this computer's server for your computers.
    fn load(&mut self, cx: &mut Context<Self>) {
        let Some(home) = self.state.read(cx).home.clone() else {
            return;
        };
        self.failed = false;
        let http = self.http.clone();
        let task = cx.background_spawn({
            let home = home.clone();
            async move { http.computers(&home) }
        });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                match result {
                    Ok(computers) => {
                        this.list = Some(ComputerList {
                            computers,
                            home: home.base,
                        })
                    }
                    Err(_) => this.failed = true,
                }
                this.loading = None;
                cx.notify();
            });
        }));
    }

    pub fn toggle(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open = !self.open;
        if self.open {
            self.focus.focus(window, cx);
            self.load(cx);
        }
        cx.notify();
    }

    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        cx.notify();
    }

    /// Opens `entry` in the window, as desktop.open(url) did.
    pub fn open_computer(&mut self, entry: &Entry, window: &mut Window, cx: &mut Context<Self>) {
        self.open = false;
        cx.notify();
        let state = self.state.read(cx);
        let current = state
            .server
            .as_ref()
            .is_some_and(|server| server.base == origin(&entry.url));
        let Some(home) = state.home.clone() else {
            return;
        };
        if current {
            return;
        }
        if entry.home {
            self.relink(Ok(home), window, cx);
            return;
        }
        if entry.via != Some(ComputerVia::Linker) {
            self.relink(ServerAddress::new(&entry.url, Auth::Local), window, cx);
            self.watch(window, cx);
            return;
        }
        let http = self.http.clone();
        let url = origin(&entry.url);
        let task = cx.background_spawn({
            let url = url.clone();
            async move { http.connect_session(&home, &url) }
        });
        cx.spawn_in(window, async move |this, cx| {
            let session = task.await;
            let _ = this.update_in(cx, |this, window, cx| match session {
                Ok(session) => {
                    this.relink(ServerAddress::new(&url, Auth::Bearer(session)), window, cx);
                    this.watch(window, cx);
                }
                Err(error) => {
                    drop(window.prompt(
                        PromptLevel::Warning,
                        "Couldn't open that computer.",
                        Some(&error),
                        &["OK"],
                        cx,
                    ));
                }
            });
        })
        .detach();
    }

    fn relink(
        &mut self,
        server: anyhow::Result<ServerAddress>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match server.and_then(|server| (self.connect)(&server)) {
            Ok(link) => self.state.update(cx, |state, cx| state.relink(link, cx)),
            Err(error) => {
                drop(window.prompt(
                    PromptLevel::Warning,
                    "Couldn't open that computer.",
                    Some(&error.to_string()),
                    &["OK"],
                    cx,
                ));
            }
        }
    }

    /// Back home unless the computer answers in time, as main.ts's did-fail-load.
    fn watch(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.reaching = Some(cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(REACH_TIMEOUT).await;
            let _ = this.update_in(cx, |this, window, cx| {
                let state = this.state.read(cx);
                if state.away() && !state.connected {
                    this.go_home(
                        "Couldn't reach that computer.",
                        "Is it on, with Linker-Edit running and remote access or Linker Connect on?",
                        window,
                        cx,
                    );
                }
            });
        }));
    }

    fn go_home(&mut self, title: &str, detail: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.reaching = None;
        if let Some(home) = self.state.read(cx).home.clone() {
            self.relink(Ok(home), window, cx);
        }
        drop(window.prompt(PromptLevel::Warning, title, Some(detail), &["OK"], cx));
    }
}

impl Render for ComputerPicker {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let state = self.state.read(cx);
        let Some(app) = state.app.as_ref() else {
            return div().into_any_element();
        };
        let computer = app.meta.computer.clone();
        let remote_on = matches!(app.meta.remote, RemoteAccess::On { .. });
        let resolve_here = app.meta.resolve_here;
        let away = state.away();
        let desktop = state.home.is_some();
        let here = state.server.as_ref().map(|server| server.base.clone());

        let accent = theme.accent;
        let toggle = div()
            .id("computer-picker")
            .flex()
            .items_center()
            .gap_2()
            .h_8()
            .w_full()
            .px_2p5()
            .rounded(theme.radius.lg)
            .text_token(TextSize::Xs)
            .cursor_pointer()
            .hover(move |style| style.bg(accent))
            .on_hover(cx.listener(|this, hovered: &bool, _, _| this.on_toggle = *hovered))
            .on_click(cx.listener(|this, _, window, cx| this.toggle(window, cx)))
            .child(
                Icon::new(IconName::Monitor)
                    .size(px(14.))
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_weight(FontWeight::MEDIUM)
                    .child(computer.clone()),
            )
            .when(away, |this| {
                this.child(
                    div()
                        .id("computer-remote")
                        .flex_shrink_0()
                        .rounded(theme.radius.sm)
                        .border_1()
                        .border_color(theme.border)
                        .px_1()
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .child("remote")
                        .test_support(),
                )
            })
            .when(remote_on, |this| {
                this.child(
                    div()
                        .size_1p5()
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(theme.success),
                )
            })
            .child(
                Icon::new(IconName::ChevronsUpDown)
                    .size(px(14.))
                    .text_color(theme.muted_foreground),
            )
            .test_support();

        let menu = self.open.then(|| {
            let row = |id: SharedString| {
                div()
                    .id(id)
                    .flex()
                    .items_center()
                    .gap_2()
                    .h_8()
                    .w_full()
                    .px_2()
                    .rounded(theme.radius.md)
                    .text_token(TextSize::Sm)
            };
            let mut items: Vec<gpui::AnyElement> = Vec::new();
            if desktop {
                items.push(
                    div()
                        .px_2()
                        .pt_1()
                        .pb_1p5()
                        .text_token(TextSize::X2s)
                        .text_color(theme.muted_foreground)
                        .child("Your computers")
                        .into_any_element(),
                );
                let entries = self.list.as_ref().map(ComputerList::entries).unwrap_or_default();
                let newest = entries.iter().fold("0.0.0".to_string(), |max, entry| {
                    if !entry.version.is_empty() && is_newer(&entry.version, &max) {
                        entry.version.clone()
                    } else {
                        max
                    }
                });
                for (index, entry) in entries.into_iter().enumerate() {
                    let current = here.as_deref() == Some(origin(&entry.url).as_str());
                    let offline = !entry.online;
                    let badge = if offline {
                        Some(
                            div()
                                .flex_shrink_0()
                                .text_token(TextSize::X2s)
                                .text_color(theme.muted_foreground)
                                .child("offline"),
                        )
                    } else if !entry.version.is_empty() && is_newer(&newest, &entry.version) {
                        Some(
                            div()
                                .flex_shrink_0()
                                .rounded(theme.radius.sm)
                                .border_1()
                                .border_color(theme.warning.opacity(0.4))
                                .px_1()
                                .text_token(TextSize::X2s)
                                .text_color(theme.warning)
                                .child(release_name(&entry.version)),
                        )
                    } else if !entry.version.is_empty() {
                        Some(
                            div()
                                .flex_shrink_0()
                                .text_token(TextSize::X2s)
                                .text_color(theme.muted_foreground)
                                .child(release_name(&entry.version)),
                        )
                    } else {
                        None
                    };
                    let tip = (entry.via == Some(ComputerVia::Linker)).then_some(if offline {
                        "Through Linker Connect: offline, or Linker-Edit isn't running there"
                    } else {
                        "Through Linker Connect"
                    });
                    let home = entry.home;
                    let name = entry.name.clone();
                    items.push(
                        row(SharedString::from(format!("computer-{index}")))
                            .map(|this| {
                                if offline {
                                    this.opacity(0.5)
                                } else {
                                    this.cursor_pointer()
                                        .hover(move |style| style.bg(accent))
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            this.open_computer(&entry, window, cx)
                                        }))
                                }
                            })
                            .when_some(tip, |this, tip| {
                                this.tooltip(move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(tip)
                                        .build(window, cx)
                                })
                            })
                            .child(
                                Icon::new(IconName::Monitor)
                                    .size(px(14.))
                                    .text_color(theme.muted_foreground),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .child(name),
                            )
                            .children(badge)
                            .when(home, |this| {
                                this.child(
                                    div()
                                        .flex_shrink_0()
                                        .text_token(TextSize::X2s)
                                        .text_color(theme.muted_foreground)
                                        .child("this computer"),
                                )
                            })
                            .when(current, |this| {
                                this.child(Icon::new(IconName::Check).size(px(14.)))
                            })
                            .test_support()
                            .into_any_element(),
                    );
                }
                let note = if self.failed {
                    Some((
                        "Couldn't look for your computers.".to_string(),
                        theme.destructive,
                    ))
                } else if self.list.is_none() {
                    Some((
                        "Looking for your computers…".to_string(),
                        theme.muted_foreground,
                    ))
                } else if self
                    .list
                    .as_ref()
                    .is_some_and(|list| list.computers.others.is_empty())
                {
                    Some(("No other computers found. On the computer with Resolve, turn on Linker Connect in its settings, and sign in to the same Linker account on both. Or use Tailscale.".to_string(), theme.muted_foreground))
                } else {
                    None
                };
                if let Some((text, color)) = note {
                    items.push(
                        div()
                            .px_2()
                            .py_1p5()
                            .text_token(TextSize::Xs)
                            .text_color(color)
                            .child(text)
                            .into_any_element(),
                    );
                }
                items.push(
                    div()
                        .my_1()
                        .border_t_1()
                        .border_color(theme.border)
                        .into_any_element(),
                );
            }
            items.push(
                row("remote-access".into())
                    .cursor_pointer()
                    .hover(move |style| style.bg(accent))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.close(cx);
                        cx.emit(ComputerPickerEvent::RemoteAccess);
                    }))
                    .child(Icon::new(IconName::Globe).size(px(14.)))
                    .child(if away {
                        format!("Remote access on {computer}…")
                    } else {
                        "Remote access…".to_string()
                    })
                    .test_support()
                    .into_any_element(),
            );
            if !resolve_here {
                let state = self.state.clone();
                items.push(
                    row("set-up-resolve-here".into())
                        .cursor_pointer()
                        .hover(move |style| style.bg(accent))
                        .tooltip(|window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(
                                "Shows Resolve's setup in the sidebar again, and downloads the speech engine here.",
                            )
                            .build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.close(cx);
                            state.read(cx).send(ClientMessage::SetResolveHere { resolve_here: true });
                        }))
                        .child(Icon::new(IconName::Wrench).size(px(14.)))
                        .child("Set up Resolve here")
                        .test_support()
                        .into_any_element(),
                );
            }
            div().absolute().top_full().left_0().mt_1().child(deferred(
                anchored().position(point(px(0.), px(0.))).position_mode(gpui::AnchoredPositionMode::Local).child(
                    div()
                        .id("computer-menu")
                        .w(px(240.))
                        .flex()
                        .flex_col()
                        .rounded(theme.radius.lg)
                        .border_1()
                        .border_color(theme.border)
                        .bg(theme.card)
                        .p_1()
                        .shadow_lg()
                        .occlude()
                        .track_focus(&self.focus)
                        .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, _, cx| {
                            if event.keystroke.key == "escape" {
                                this.close(cx);
                            }
                        }))
                        .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                            if !this.on_toggle {
                                this.close(cx);
                            }
                        }))
                        .children(items)
                        .test_support(),
                ),
            ).with_priority(1))
        });

        div()
            .id("computer-menu-root")
            .relative()
            .mx_2()
            .mb_1()
            .child(toggle)
            .children(menu)
            .test_support()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_code_out_of_the_address_that_swaps_it() {
        assert_eq!(
            code_of("https://edit-0123456789abcdef.linker.cool/api/connect/enter?code=a%2Bb-c_d")
                .as_deref(),
            Some("a+b-c_d")
        );
        assert_eq!(code_of("https://x/api/connect/enter"), None);
    }
}
