//! The GPUI window: the desktop UI of Linker-Edit, one client of the server like the browser.
//! See README.md in this crate for how views, state, the theme and icons fit together.

// gpui-kit re-exports GPUI; GPUI's macros expand to `gpui::…`, so it goes by that name too.
extern crate gpui_kit as gpui;

pub mod browse_path;
pub mod collection_icons;
pub mod components;
pub mod context;
pub mod http;
pub mod icons;
pub mod labels;
pub mod library;
pub mod markdown;
pub mod media;
#[cfg(test)]
mod pages_tests;
pub mod project_icon_colors;
pub mod settings;
pub mod sidebar;
pub mod single_instance;
pub mod state;
pub mod theme;
pub mod transport;
pub mod views;
pub mod window;
pub mod workspace;

use futures::StreamExt;
use gpui::{AppContext, Entity, Window};

pub use state::{
    AppEvent, AppState, Auth, ClientMessage, Link, LinkEvent, ServerAddress, ServerEnd,
    ServerMessage,
};
pub use workspace::Workspace;

/// Opens the window on `link` and runs until it closes. The link's server (`Link::server`) is
/// this computer's: pictures and media come from it, the computer menu asks it for the user's
/// other computers and comes back to it. If another copy already runs on the same data folder,
/// brings its window forward and returns at once. `on_quit` runs once the window closed, before the app quits:
/// the place to install a downloaded update, since on macOS quitting ends the process.
pub fn run(link: Link, on_quit: impl FnOnce() + 'static) -> anyhow::Result<()> {
    let mut focus_requests = match single_instance::claim()? {
        single_instance::Instance::Primary(requests) => requests,
        single_instance::Instance::Secondary => return Ok(()),
    };

    gpui_kit::application()
        .with_assets(icons::Assets)
        .run(move |cx| {
            gpui_kit::init(cx);
            if let Err(error) = http::install(cx) {
                eprintln!("No pictures from the server: {error}");
            }
            let state = AppState::new(link, cx);
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
            let mut on_quit = Some(on_quit);
            cx.on_window_closed(move |cx, _| {
                if let Some(on_quit) = on_quit.take() {
                    on_quit();
                }
                cx.quit();
            })
            .detach();
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
    workspace::bind_keys(cx);
    cx.new(|cx| Workspace::new(state, window, cx))
}
