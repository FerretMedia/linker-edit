//! The root view (+page.svelte's main layout): the sidebar, the chat column with its header,
//! and at most one side panel. It owns what is only the window's business (which panel is
//! open); everything else comes from [`AppState`].

use gpui::{
    AppContext, Context, Entity, FontWeight, InteractiveElement, IntoElement, ParentElement,
    Render, Styled, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;

use crate::components::HeaderButton;
use crate::icons::IconName;
use crate::state::AppState;
use crate::theme::{TextSize, TextStyled, Theme, root_text};
use crate::views::{Chat, Panel, SidePanel, Sidebar};
use crate::window::{WindowControls, no_drag, title_bar};

pub struct Workspace {
    state: Entity<AppState>,
    sidebar: Entity<Sidebar>,
    chat: Entity<Chat>,
    panel: Option<Panel>,
}

impl Workspace {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        cx.observe(&state, |_, _, cx| cx.notify()).detach();
        Workspace {
            sidebar: cx.new(|cx| Sidebar::new(state.clone(), cx)),
            chat: cx.new(|cx| Chat::new(state.clone(), cx)),
            state,
            panel: None,
        }
    }

    pub fn panel(&self) -> Option<Panel> {
        self.panel
    }

    /// Opens `panel`, or closes it when it is the open one.
    pub fn toggle_panel(&mut self, panel: Panel, cx: &mut Context<Self>) {
        self.panel = (self.panel != Some(panel)).then_some(panel);
        cx.notify();
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let state = self.state.read(cx);
        let lost = state.ever_connected && !state.connected;

        let toggles = Panel::ALL.map(|panel| {
            HeaderButton::new(panel.id(), panel.icon(), panel.label())
                .pressed(self.panel == Some(panel))
                .on_click(cx.listener(move |this, _, _, cx| this.toggle_panel(panel, cx)))
        });
        let header = title_bar("chat-header", px(20.), false, self.panel.is_none(), window)
            .gap_2()
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_token(TextSize::Sm)
                    .font_weight(FontWeight::MEDIUM)
                    .child("New thread"),
            )
            .child(
                no_drag()
                    .ml_auto()
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .gap_1p5()
                    .child(HeaderButton::new("library", IconName::Library, "Library"))
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
                        .child("Lost connection to the Linker-Edit server. Reconnecting…")
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
            .child(self.sidebar.clone())
            .child(main)
            .when_some(self.panel, |this, panel| {
                this.child(SidePanel::new(
                    panel,
                    cx.listener(|this, _, _, cx| {
                        this.panel = None;
                        cx.notify();
                    }),
                ))
            })
            .child(WindowControls)
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use futures::channel::mpsc::UnboundedSender;
    use gpui::{
        AnyWindowHandle, AppContext, Bounds, TestAppContext, WindowBounds, WindowOptions, point,
        px, size,
    };
    use gpui_kit::test::TestWindowExt;

    use crate::state::{Link, LinkEvent};

    /// The window as `run` opens it, at main.ts's size, on a link the test drives.
    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, UnboundedSender<LinkEvent>) {
        cx.update(gpui_kit::init);
        let (link, server) = Link::pair();
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
                crate::new_workspace(state, window, cx)
            })
            .expect("the window opens")
            .0
        });
        (window, server.events)
    }

    #[gpui::test]
    fn lays_out_the_columns_and_toggles_a_side_panel(cx: &mut TestAppContext) {
        let (window, _server) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.find("sidebar").bounds().size.width, px(256.));
            assert!(window.find("chat").visible());
            assert!(window.try_find("preview-panel").is_none());

            window.click("preview", cx);
            assert_eq!(window.find("preview-panel").bounds().size.width, px(512.));

            window.click("close-panel", cx);
            assert!(window.try_find("preview-panel").is_none());
        })
        .unwrap();
    }

    #[gpui::test]
    fn says_so_when_the_link_to_the_server_drops(cx: &mut TestAppContext) {
        let (window, server) = open(cx);
        let banner = |cx: &mut TestAppContext| {
            cx.run_until_parked();
            cx.update_window(window, |_, window, cx| {
                window.render_frame(cx);
                window.try_find("lost-connection").is_some()
            })
            .unwrap()
        };

        // The first wait is "connecting", not a lost connection.
        assert!(!banner(cx));
        server.unbounded_send(LinkEvent::Connected).unwrap();
        assert!(!banner(cx));
        server.unbounded_send(LinkEvent::Disconnected).unwrap();
        assert!(banner(cx));
        server.unbounded_send(LinkEvent::Connected).unwrap();
        assert!(!banner(cx));
    }
}
