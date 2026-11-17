//! The window's frame, like desktop/main.ts: no system title bar; the app's top headers are the
//! title bar and drag the window. Windows and Linux get minimize/maximize/close drawn over the
//! top right corner; macOS keeps its traffic lights, inset into the sidebar's header.
//!
//! - Build a header with [`title_bar`]; it is 52px high (main.ts's TITLE_BAR_HEIGHT, h-13).
//! - Wrap every control inside a header in [`no_drag`], or pressing it drags the window.
//! - Tell it whether it is the row's leftmost header (it clears macOS's traffic lights) and
//!   rightmost one (it clears the window buttons); its padding makes the room.

use gpui::{
    App, Bounds, Div, ElementId, Global, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, RenderOnce, SharedString, Stateful, StatefulInteractiveElement, Styled,
    TitlebarOptions, Window, WindowBounds, WindowControlArea, WindowOptions, div, point,
    prelude::FluentBuilder, px, size,
};

use gpui_kit::{assets::IconName as KitIcon, component::Icon};

use crate::theme::Theme;

pub const TITLE: &str = "Linker-Edit";
/// main.ts's TITLE_BAR_HEIGHT: the headers' h-13.
pub const TITLE_BAR_HEIGHT: Pixels = px(52.);
/// Windows' caption buttons, as Electron's title bar overlay draws them.
const CONTROL_WIDTH: Pixels = px(46.);
/// Space the rightmost header leaves for the window buttons (app.css: their width + 0.75rem).
fn controls_inset() -> Pixels {
    CONTROL_WIDTH * 3. + px(12.)
}

/// main.ts's window: 1320 × 860, at least 900 × 560, centered on the main display.
/// It doesn't remember its size or place between runs; neither does the Electron one.
pub fn options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(1320.), px(860.)),
            cx,
        ))),
        window_min_size: Some(size(px(900.), px(560.))),
        titlebar: Some(TitlebarOptions {
            title: Some(TITLE.into()),
            appears_transparent: true,
            traffic_light_position: Some(point(px(18.), px(19.))),
        }),
        // The headers move the window themselves on macOS (see `title_bar`).
        app_owns_titlebar_drag: true,
        app_id: Some("linker-edit".into()),
        ..Default::default()
    }
}

/// A header that is also the title bar: drags the window, and double-clicking maximizes it.
/// `padding_x` is its own side padding (px-4 is 16px); don't set padding on it afterwards.
pub fn title_bar(
    id: impl Into<ElementId>,
    padding_x: Pixels,
    leftmost: bool,
    rightmost: bool,
    window: &Window,
) -> Stateful<Div> {
    let mac = cfg!(target_os = "macos");
    // macOS's traffic lights sit in the leftmost header (app.css's 5.5rem fallback + 0.75rem);
    // in full screen they hide and the content moves back.
    let left = if mac && leftmost && !window.is_fullscreen() {
        px(100.)
    } else {
        padding_x
    };
    // Windows' and Linux's buttons sit over the rightmost header's corner.
    let right = if !mac && rightmost {
        controls_inset()
    } else {
        padding_x
    };
    div()
        .id(id)
        .flex()
        .flex_row()
        .items_center()
        .flex_shrink_0()
        .h(TITLE_BAR_HEIGHT)
        .pl(left)
        .pr(right)
        // Windows does the drag, double-click and Snap itself for a caption area.
        .when(cfg!(target_os = "windows"), |this| {
            this.window_control_area(WindowControlArea::Drag)
        })
        .when(!cfg!(target_os = "windows"), |this| {
            this.on_mouse_down(MouseButton::Left, |event, window, _| {
                if event.click_count == 1 {
                    window.start_window_move();
                }
            })
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    if cfg!(target_os = "macos") {
                        window.titlebar_double_click();
                    } else {
                        window.zoom_window();
                    }
                }
            })
        })
}

/// A group of controls inside a header. It takes the mouse, so the header doesn't drag
/// the window from under a button (on Windows, the caption hit test skips it).
pub fn no_drag() -> Div {
    div().occlude()
}

/// Minimize, maximize and close over the window's top right corner, on Windows and Linux.
/// The workspace draws it once, above everything.
#[derive(IntoElement)]
pub struct WindowControls;

#[derive(Clone, Copy, PartialEq)]
enum Control {
    Minimize,
    Maximize,
    Restore,
    Close,
}

impl RenderOnce for WindowControls {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        if cfg!(target_os = "macos") {
            return div().into_any_element();
        }
        // On Linux with server-side decorations the window manager draws its own.
        #[cfg(target_os = "linux")]
        if !matches!(
            window.window_decorations(),
            gpui::Decorations::Client { .. }
        ) {
            return div().into_any_element();
        }
        let maximize = if window.is_maximized() {
            Control::Restore
        } else {
            Control::Maximize
        };
        div()
            .id("window-controls")
            .absolute()
            .top_0()
            .right_0()
            .h(TITLE_BAR_HEIGHT)
            .flex()
            .flex_row()
            .child(control(Control::Minimize, cx))
            .child(control(maximize, cx))
            .child(control(Control::Close, cx))
            .into_any_element()
    }
}

fn control(kind: Control, cx: &mut App) -> impl IntoElement {
    let theme = Theme::of(cx);
    // main.ts's symbolColor.
    let symbol: Hsla = if theme.dark {
        gpui::rgb(0xf5f5f5).into()
    } else {
        gpui::rgb(0x27272a).into()
    };
    let close_red: Hsla = gpui::rgb(0xc42b1c).into();
    let (hover_bg, hover_fg) = if kind == Control::Close {
        (close_red, gpui::white())
    } else {
        (symbol.opacity(0.08), symbol)
    };
    let (id, glyph, icon, area) = match kind {
        Control::Minimize => (
            "minimize",
            '\u{e921}',
            KitIcon::WindowMinimize,
            WindowControlArea::Min,
        ),
        Control::Maximize => (
            "maximize",
            '\u{e922}',
            KitIcon::WindowMaximize,
            WindowControlArea::Max,
        ),
        Control::Restore => (
            "restore",
            '\u{e923}',
            KitIcon::WindowRestore,
            WindowControlArea::Max,
        ),
        Control::Close => (
            "close",
            '\u{e8bb}',
            KitIcon::WindowClose,
            WindowControlArea::Close,
        ),
    };
    let windows = cfg!(target_os = "windows");
    div()
        .id(id)
        .occlude()
        .w(CONTROL_WIDTH)
        .h_full()
        .flex()
        .items_center()
        .justify_center()
        .text_color(symbol)
        .hover(move |style| style.bg(hover_bg).text_color(hover_fg))
        .active(move |style| style.bg(hover_bg.opacity(0.9)).text_color(hover_fg))
        // Windows' own caption glyphs, so the buttons look native; Windows handles their
        // clicks itself (and shows Snap layouts over maximize).
        .when(windows, |this| {
            this.font_family(glyph_font(cx))
                .text_size(px(10.))
                .child(SharedString::from(glyph.to_string()))
                .window_control_area(area)
        })
        .when(!windows, |this| {
            this.child(Icon::new(icon).size(px(16.)))
                .on_click(move |_, window, _| match kind {
                    Control::Minimize => window.minimize_window(),
                    Control::Maximize | Control::Restore => window.zoom_window(),
                    Control::Close => window.remove_window(),
                })
        })
}

/// The caption glyphs' font: Segoe Fluent Icons on Windows 11, Segoe MDL2 Assets on 10.
fn glyph_font(cx: &mut App) -> SharedString {
    struct GlyphFont(SharedString);
    impl Global for GlyphFont {}
    if let Some(font) = cx.try_global::<GlyphFont>() {
        return font.0.clone();
    }
    let fonts = cx.text_system().all_font_names();
    let font: SharedString = if fonts.iter().any(|name| name == "Segoe Fluent Icons") {
        "Segoe Fluent Icons".into()
    } else {
        "Segoe MDL2 Assets".into()
    };
    cx.set_global(GlyphFont(font.clone()));
    font
}
