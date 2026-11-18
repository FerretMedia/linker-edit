//! The GPUI window: the desktop UI of Linker-Edit, one client of the server like the browser.
//! See README.md in this crate for how views, state, the theme and icons fit together.

// gpui-kit re-exports GPUI; GPUI's macros expand to `gpui::…`, so it goes by that name too.
extern crate gpui_kit as gpui;

pub mod browse_path;
pub mod collection_icons;
pub mod components;
pub mod context;
pub mod icons;
pub mod labels;
pub mod markdown;
pub mod project_icon_colors;
pub mod sidebar;
mod single_instance;
pub mod state;
pub mod theme;
pub mod transport;
pub mod views;
pub mod window;
pub mod workspace;

use futures::StreamExt;
use gpui::{AppContext, Entity, Window};

pub use state::{AppEvent, AppState, ClientMessage, Link, LinkEvent, ServerEnd, ServerMessage};
pub use workspace::Workspace;

/// Opens the window on `link` and runs until it closes. `home` is the http(s) address of the
/// server `link` goes to, this computer's: the computer menu asks it for the user's other
/// computers and comes back to it. If another copy already runs on the same data folder, brings
/// its window forward and returns at once.
pub fn run(link: Link, home: Option<String>) -> anyhow::Result<()> {
    let mut focus_requests = match single_instance::claim()? {
        single_instance::Instance::Primary(requests) => requests,
        single_instance::Instance::Secondary => return Ok(()),
    };

    gpui_kit::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            let state = AppState::new(link, cx);
            state.update(cx, |state, _| {
                state.server = home.clone();
                state.home = home;
            });
            let (window, _) = gpui_kit::open_window(window::options(cx), cx, |window, cx| {
                new_workspace(state, window, cx)
            })
            .expect("couldn't open the window");

            cx.spawn(async move |cx| {
                while focus_requests.next().await.is_some() {
                    let _ = window.update(cx, |_, window, _| window.activate_window());
                }
            })
            .detach();
            // One window: closing it quits, as main.ts's window-all-closed does.
            cx.on_window_closed(|cx, _| cx.quit()).detach();
        });
    Ok(())
}

/// The root view with the theme installed: what `run` opens, and what tests render.
pub fn new_workspace(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut gpui::App,
) -> Entity<Workspace> {
    theme::Theme::init(window, cx);
    cx.new(|cx| Workspace::new(state, window, cx))
}
