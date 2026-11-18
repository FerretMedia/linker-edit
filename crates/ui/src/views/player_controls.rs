//! PlayerControls.svelte: the scrubber with the clips and markers on it, and the row of buttons
//! under the picture (cuts, play, the timecode, mute, volume, full screen). White on the dark
//! stage in both themes, as in the browser, so its colors are the Svelte's literal ones.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::{
    App, Bounds, Corners, Hitbox, HitboxBehavior, Hsla, InteractiveElement, IntoElement,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement, PathBuilder, Pixels,
    RenderImage, RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div,
    point, prelude::FluentBuilder, px, relative, rgb,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::tooltip::Tooltip;
use linker_edit_protocol::{MediaKind, PlayableTimeline};

use crate::icons::IconName;
use crate::media::timeline::format_timecode;
use crate::theme::{TextSize, TextStyled, Theme};

/// What the controls ask the panel to do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Control {
    Seek(f64),
    /// The pointer over the scrubber: where, and the time there.
    Hover(Option<(Pixels, f64)>),
    Drag(Option<Drag>),
    TogglePlay,
    /// The previous (-1) or next (1) cut.
    Cut(i8),
    ToggleMute,
    Volume(f32),
    ToggleFullscreen,
}

/// What a held mouse button is moving.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drag {
    Position,
    Volume,
}

pub type OnControl = Rc<dyn Fn(Control, &mut Window, &mut App)>;

pub fn white(alpha: f32) -> Hsla {
    gpui::white().opacity(alpha)
}

pub fn black(alpha: f32) -> Hsla {
    gpui::black().opacity(alpha)
}

/// Tailwind 4's amber-400 and sky-500.
fn amber_400() -> Hsla {
    rgb(0xffb900).into()
}

fn sky_500() -> Hsla {
    rgb(0x00a6f4).into()
}

/// Resolve's marker colours.
fn marker_color(name: &str) -> Hsla {
    rgb(match name {
        "Blue" => 0x3b82f6,
        "Cyan" => 0x22d3ee,
        "Green" => 0x22c55e,
        "Yellow" => 0xfacc15,
        "Red" => 0xef4444,
        "Pink" => 0xf472b6,
        "Purple" => 0xa855f7,
        "Fuchsia" => 0xd946ef,
        "Rose" => 0xfb7185,
        "Lavender" => 0xc4b5fd,
        "Sky" => 0x7dd3fc,
        "Mint" => 0x6ee7b7,
        "Lemon" => 0xfef08a,
        "Sand" => 0xd6c29c,
        "Cocoa" => 0xa16207,
        "Cream" => 0xfef3c7,
        _ => 0xffffff,
    })
    .into()
}

#[derive(IntoElement)]
pub struct PlayerControls {
    /// Draws the clips and markers on the scrubber; None shows a plain bar.
    pub timeline: Option<PlayableTimeline>,
    /// Image clips' pictures on the scrubber, by media id.
    pub thumbnails: HashMap<String, Arc<RenderImage>>,
    pub duration: f64,
    pub time: f64,
    pub playing: bool,
    pub muted: bool,
    pub volume: f32,
    pub fullscreen: bool,
    pub hover: Option<(Pixels, f64)>,
    pub drag: Option<Drag>,
    pub on: OnControl,
}

impl PlayerControls {
    fn fraction(&self, seconds: f64) -> f32 {
        if self.duration > 0. {
            (seconds / self.duration).clamp(0., 1.) as f32
        } else {
            0.
        }
    }

    fn scrubber(&self) -> impl IntoElement {
        let timeline = self.timeline.as_ref();
        let clips = timeline.map_or(&[][..], |timeline| &timeline.video[..]);
        let clips = clips.iter().enumerate().map(|(index, clip)| {
            let image = (clip.track == 1 && clip.kind == MediaKind::Image)
                .then(|| {
                    clip.media
                        .as_ref()
                        .and_then(|media| self.thumbnails.get(media))
                })
                .flatten()
                .cloned();
            let name = SharedString::from(clip.name.clone());
            div()
                .id(("scrubber-clip", index))
                .absolute()
                .left(relative(self.fraction(clip.start)))
                .w(relative(self.fraction(clip.end - clip.start)))
                .overflow_hidden()
                .rounded(px(3.))
                .border_1()
                .border_color(black(0.6))
                .map(|this| {
                    if clip.track > 1 {
                        this.top_0().h_2().bg(amber_400().opacity(0.7))
                    } else {
                        this.top_0().bottom_0()
                    }
                })
                .when(clip.track == 1 && clip.kind == MediaKind::Video, |this| {
                    this.bg(sky_500().opacity(0.35))
                })
                .when(
                    clip.track == 1
                        && matches!(clip.kind, MediaKind::Missing | MediaKind::Unsupported),
                    |this| this.bg(white(0.1)),
                )
                .when_some(image, |this, image| this.child(cover(image)))
                .tooltip(move |window, cx| Tooltip::new(name.clone()).build(window, cx))
                .test_support()
        });
        let markers: Vec<(f32, Hsla)> = timeline
            .map(|timeline| {
                timeline
                    .markers
                    .iter()
                    .map(|marker| (self.fraction(marker.time), marker_color(&marker.color)))
                    .collect()
            })
            .unwrap_or_default();
        let at = self.fraction(self.time);
        let fps = timeline.map_or(24., |timeline| timeline.fps);
        let (duration, drag, hovering) = (self.duration, self.drag, self.hover.is_some());
        let on = self.on.clone();

        div()
            .id("scrubber")
            .relative()
            .h_9()
            .cursor_pointer()
            .overflow_hidden()
            .rounded(px(6.))
            .bg(white(0.08))
            .children(clips)
            // What is still to come is darker.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(at))
                    .right_0()
                    .bg(black(0.45)),
            )
            .child(
                canvas(
                    |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |bounds, hitbox, window, _| {
                        for (left, color) in markers {
                            diamond(window, bounds, left, color);
                        }
                        track_pointer(window, bounds, hitbox, drag, hovering, on, move |x| {
                            (x as f64) * duration
                        });
                    },
                )
                .absolute()
                .size_full(),
            )
            // The playhead, with a dark ring.
            .child(
                div()
                    .id("playhead")
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(relative(at))
                    .ml(px(-2.))
                    .w(px(4.))
                    .flex()
                    .justify_center()
                    .bg(black(0.4))
                    .child(div().w_0p5().h_full().bg(gpui::white()))
                    .test_support(),
            )
            .when_some(self.hover, |this, (x, time)| {
                // Centered on the pointer, kept 1.5rem from the ends.
                this.child(
                    div()
                        .absolute()
                        .bottom_0p5()
                        .left(x - px(32.))
                        .w(px(64.))
                        .flex()
                        .justify_center()
                        .child(
                            div()
                                .id("scrubber-hover")
                                .rounded(px(4.))
                                .bg(black(0.8))
                                .px_1()
                                .text_size(px(10.))
                                .line_height(px(16.))
                                .child(format_timecode(time, fps))
                                .test_support(),
                        ),
                )
            })
            .test_support()
    }

    fn volume_slider(&self) -> impl IntoElement {
        let value = if self.muted {
            0.
        } else {
            self.volume.clamp(0., 1.)
        };
        let (drag, on) = (self.drag, self.on.clone());
        // A range input with accent-white: the filled part white, a round white thumb.
        div()
            .id("volume")
            .relative()
            .w_16()
            .h_4()
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(px(6.))
                    .h_1()
                    .rounded_full()
                    .bg(white(0.3)),
            )
            .child(
                div()
                    .absolute()
                    .left_0()
                    .top(px(6.))
                    .h_1()
                    .w(relative(value))
                    .rounded_full()
                    .bg(gpui::white()),
            )
            .child(
                div()
                    .absolute()
                    .top(px(1.))
                    .left(px(value * (64. - 14.)))
                    .size(px(14.))
                    .rounded_full()
                    .bg(gpui::white()),
            )
            .child(
                canvas(
                    |bounds, window, _| window.insert_hitbox(bounds, HitboxBehavior::Normal),
                    move |bounds, hitbox, window, _| {
                        let dragging = (drag == Some(Drag::Volume)).then_some(Drag::Volume);
                        track_volume(window, bounds, hitbox, dragging, on);
                    },
                )
                .absolute()
                .size_full(),
            )
            .tooltip(|window, cx| Tooltip::new("Volume").build(window, cx))
            .test_support()
    }
}

/// An image filling `background: center / cover` in its parent.
fn cover(image: Arc<RenderImage>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let size = image.size(0);
            let (width, height) = (size.width.0 as f32, size.height.0 as f32);
            if width == 0. || height == 0. {
                return;
            }
            let scale =
                (f32::from(bounds.size.width) / width).max(f32::from(bounds.size.height) / height);
            let drawn = gpui::size(px(width * scale), px(height * scale));
            let image_bounds = Bounds::centered_at(bounds.center(), drawn);
            let _ = window.paint_image(bounds, image_bounds, Corners::default(), image, 0, false);
        },
    )
    .size_full()
}

/// A marker: an 8px square turned 45°, its top a pixel above the bar.
fn diamond(window: &mut Window, bounds: Bounds<Pixels>, left: f32, color: Hsla) {
    let x = bounds.origin.x + bounds.size.width * left;
    let y = bounds.origin.y + px(3.);
    let half = px(4. * std::f32::consts::SQRT_2);
    let mut path = PathBuilder::fill();
    path.move_to(point(x, y - half));
    path.line_to(point(x + half, y));
    path.line_to(point(x, y + half));
    path.line_to(point(x - half, y));
    path.close();
    if let Ok(path) = path.build() {
        window.paint_path(path, color);
    }
}

/// The scrubber's pointer: press to seek and drag on (anywhere in the window, as the browser's
/// pointer capture), hover to see the time there.
fn track_pointer(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    hitbox: Hitbox,
    drag: Option<Drag>,
    hovering: bool,
    on: OnControl,
    time_at: impl Fn(f32) -> f64 + Clone + 'static,
) {
    let at = move |position: gpui::Point<Pixels>| {
        let x = (position.x - bounds.origin.x).clamp(px(0.), bounds.size.width);
        let fraction = if bounds.size.width > px(0.) {
            x / bounds.size.width
        } else {
            0.
        };
        (x, fraction)
    };
    let (down, time_down) = (on.clone(), time_at.clone());
    let hitbox_down = hitbox.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase.bubble() && event.button == MouseButton::Left && hitbox_down.is_hovered(window) {
            let (_, fraction) = at(event.position);
            down(Control::Drag(Some(Drag::Position)), window, cx);
            down(Control::Seek(time_down(fraction)), window, cx);
        }
    });
    let moved = on.clone();
    window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
        if !phase.bubble() {
            return;
        }
        let (x, fraction) = at(event.position);
        let dragging = drag == Some(Drag::Position);
        if dragging || hitbox.is_hovered(window) {
            moved(Control::Hover(Some((x, time_at(fraction)))), window, cx);
            if dragging {
                moved(Control::Seek(time_at(fraction)), window, cx);
            }
        } else if hovering {
            moved(Control::Hover(None), window, cx);
        }
    });
    if drag.is_some() {
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
            if phase.bubble() {
                on(Control::Drag(None), window, cx);
            }
        });
    }
}

/// The volume slider's pointer, in steps of 0.05 as the range input's.
fn track_volume(
    window: &mut Window,
    bounds: Bounds<Pixels>,
    hitbox: Hitbox,
    drag: Option<Drag>,
    on: OnControl,
) {
    let value = move |position: gpui::Point<Pixels>| {
        let x = (position.x - bounds.origin.x).clamp(px(0.), bounds.size.width);
        let fraction = if bounds.size.width > px(0.) {
            x / bounds.size.width
        } else {
            0.
        };
        (fraction * 20.).round() / 20.
    };
    let down = on.clone();
    window.on_mouse_event(move |event: &MouseDownEvent, phase, window, cx| {
        if phase.bubble() && event.button == MouseButton::Left && hitbox.is_hovered(window) {
            down(Control::Drag(Some(Drag::Volume)), window, cx);
            down(Control::Volume(value(event.position)), window, cx);
        }
    });
    if drag.is_some() {
        let moved = on.clone();
        window.on_mouse_event(move |event: &MouseMoveEvent, phase, window, cx| {
            if phase.bubble() {
                moved(Control::Volume(value(event.position)), window, cx);
            }
        });
        window.on_mouse_event(move |_: &MouseUpEvent, phase, window, cx| {
            if phase.bubble() {
                on(Control::Drag(None), window, cx);
            }
        });
    }
}

/// `flex size-8 items-center justify-center rounded-md hover:bg-white/10`, with its title.
fn button(
    id: &'static str,
    icon: IconName,
    title: SharedString,
    control: Control,
    on: &OnControl,
) -> impl IntoElement {
    let on = on.clone();
    div()
        .id(id)
        .flex()
        .size_8()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .rounded(px(6.))
        .cursor_pointer()
        .hover(|style| style.bg(white(0.1)))
        .child(Icon::new(icon).size(px(16.)))
        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
        .on_click(move |_, window, cx| on(control, window, cx))
        .test_support()
}

impl RenderOnce for PlayerControls {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let fps = self.timeline.as_ref().map_or(24., |timeline| timeline.fps);
        let on = &self.on;
        let (play_icon, play_title) = if self.playing {
            (IconName::PauseFilled, "Pause (Space)")
        } else {
            (IconName::PlayFilled, "Play (Space)")
        };
        let silent = self.muted || self.volume == 0.;
        let row = div()
            .flex()
            .items_center()
            .gap_0p5()
            .child(button(
                "previous-cut",
                IconName::SkipBack,
                "Previous cut (↑)".into(),
                Control::Cut(-1),
                on,
            ))
            .child(button(
                "play",
                play_icon,
                play_title.into(),
                Control::TogglePlay,
                on,
            ))
            .child(button(
                "next-cut",
                IconName::SkipForward,
                "Next cut (↓)".into(),
                Control::Cut(1),
                on,
            ))
            .child(
                div()
                    .id("player-time")
                    .ml_2()
                    .flex()
                    .font_family(theme.mono_font_family)
                    .text_token(TextSize::Xs)
                    .child(format_timecode(self.time, fps))
                    .child(
                        div()
                            .text_color(white(0.45))
                            .child(format!(" / {}", format_timecode(self.duration, fps))),
                    )
                    .test_support(),
            )
            .child(div().flex_1())
            .child(button(
                "mute",
                if silent {
                    IconName::VolumeX
                } else {
                    IconName::Volume2
                },
                if self.muted { "Unmute" } else { "Mute" }.into(),
                Control::ToggleMute,
                on,
            ))
            .child(self.volume_slider())
            .child(
                div().ml_1().child(button(
                    "fullscreen",
                    if self.fullscreen {
                        IconName::Minimize
                    } else {
                        IconName::Maximize
                    },
                    if self.fullscreen {
                        "Exit full screen (F)"
                    } else {
                        "Full screen (F)"
                    }
                    .into(),
                    Control::ToggleFullscreen,
                    on,
                )),
            );
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .px_3()
            .pt_2p5()
            .pb_2()
            .text_color(gpui::white())
            .child(self.scrubber())
            .child(row)
    }
}
