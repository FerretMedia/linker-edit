//! PreviewPanel.svelte: watch the timeline (a version from the history, or as it is in Resolve
//! now) as a cut preview straight from its media, or as Resolve renders it; render, stop, make an
//! AI timeline the main one, and start feedback at the playhead.
//!
//! The timeline comes from `/api/timeline` as the browser loads it; the picture and sound are
//! [`Player`]'s. Keys (Space play, ← → frame, ↑ ↓ cut, F full screen, M mute) work while the
//! stage has focus, as the Svelte's.

use std::cell::Cell;
use std::collections::HashMap;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, HighlightStyle, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled, StyledText,
    Subscription, Task, Window, anchored, canvas, deferred, div, point, prelude::FluentBuilder, px,
    rgb,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable};
use linker_edit_protocol::{
    ClientMessage, PlayableTimeline, PreviewState, ResolveStatus, TimelineRef,
};

use super::player_controls::{Control, Drag, OnControl, PlayerControls, black, white};
use super::timeline_player::TimelinePlayer;
use crate::http;
use crate::icons::IconName;
use crate::media::ffmpeg;
use crate::media::player::{Player, Program, encode_component};
use crate::media::timeline::{PreviewTarget, nearest_cut};
use crate::state::{AppState, ServerAddress};
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::panel::{Panel, close_button, panel_frame, panel_header};

/// What the panel asks of the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PreviewEvent {
    /// Show another version (the timeline as it is now, from "show it as it is now").
    Target(PreviewTarget),
    /// Start a message about this moment, as "At 0:14 (frame 336) in "AI - Cut"".
    Comment(String),
    Close,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Cut,
    Render,
}

/// A render the panel can play: Resolve's preview of the timeline in view.
struct Rendered {
    url: String,
    outdated: bool,
}

pub struct PreviewPanel {
    state: Entity<AppState>,
    /// The project the target is in; another project shows its current timeline.
    target: (Option<String>, PreviewTarget),
    timeline: Option<PlayableTimeline>,
    loading: bool,
    load_error: Option<String>,
    /// The project and target the timeline was asked for, and which ask it was.
    asked: Option<(Option<String>, PreviewTarget)>,
    asks: u64,
    chosen: Mode,
    player: Player,
    muted: bool,
    volume: f32,
    fullscreen: bool,
    /// Making a timeline the main one renames two timelines in Resolve, so it takes a second click.
    confirm_apply: bool,
    /// A render that was already there when the panel opened doesn't take over; one finishing now does.
    shown_render: Option<String>,
    was_busy: bool,
    /// How long the render at a URL runs, once ffmpeg said (the video element's duration).
    render_length: Option<(String, Option<f64>)>,
    hover: Option<(Pixels, f64)>,
    drag: Option<Drag>,
    focus: FocusHandle,
    /// The stage's size in device pixels, as last laid out.
    stage: Rc<Cell<(u32, u32)>>,
    /// The next draw while playing; replacing it cancels the one before.
    redraw: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<PreviewEvent> for PreviewPanel {}

impl Focusable for PreviewPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl PreviewPanel {
    pub fn new(
        state: Entity<AppState>,
        target: (Option<String>, PreviewTarget),
        cx: &mut Context<Self>,
    ) -> Self {
        let player = Player::new(state.read(cx).server.clone());
        PreviewPanel::with_player(state, target, player, cx)
    }

    /// With a player of the caller's (tests: one without a sound card).
    pub fn with_player(
        state: Entity<AppState>,
        target: (Option<String>, PreviewTarget),
        player: Player,
        cx: &mut Context<Self>,
    ) -> Self {
        let read = state.read(cx);
        let shown_render = match read.app.as_ref().map(|app| &app.meta.preview) {
            Some(PreviewState::Ready { url, .. }) => Some(url.clone()),
            _ => None,
        };
        let was_busy = read.resolve_busy();
        let mut panel = PreviewPanel {
            _subscriptions: vec![cx.observe(&state, |this, _, cx| this.on_state(cx))],
            state,
            target,
            timeline: None,
            loading: false,
            load_error: None,
            asked: None,
            asks: 0,
            chosen: Mode::Cut,
            player,
            muted: false,
            volume: 1.,
            fullscreen: false,
            confirm_apply: false,
            shown_render,
            was_busy,
            render_length: None,
            hover: None,
            drag: None,
            focus: cx.focus_handle(),
            stage: Rc::new(Cell::new((0, 0))),
            redraw: None,
        };
        panel.load(cx);
        panel
    }

    /// Shows `target` of the project `project_id`.
    pub fn set_target(&mut self, target: (Option<String>, PreviewTarget), cx: &mut Context<Self>) {
        if self.target != target {
            self.target = target;
            self.on_state(cx);
        }
    }

    /// The target, or the current timeline when the project in view isn't the target's.
    fn current(&self, cx: &App) -> (Option<String>, PreviewTarget) {
        let project = self.state.read(cx).project_id().map(str::to_string);
        if project == self.target.0 {
            (project, self.target.1.clone())
        } else {
            (project, PreviewTarget::default())
        }
    }

    pub fn timeline(&self) -> Option<&PlayableTimeline> {
        self.timeline.as_ref()
    }

    pub fn player(&self) -> &Player {
        &self.player
    }

    /// What the stage shows: a render only while one of this timeline exists.
    pub fn source(&self, cx: &App) -> Mode {
        if self.chosen == Mode::Render && self.rendered(cx).is_some() {
            Mode::Render
        } else {
            Mode::Cut
        }
    }

    /// The server changed something: load another timeline, or the same again after the agent's
    /// turn (it may have changed it), and let a render that just finished take over.
    fn on_state(&mut self, cx: &mut Context<Self>) {
        let current = self.current(cx);
        let busy = self.state.read(cx).resolve_busy();
        if self.asked.as_ref() != Some(&current)
            || (self.was_busy && !busy && current.1.commit.is_none())
        {
            self.load(cx);
        }
        self.was_busy = busy;
        if let Some(rendered) = self.rendered(cx)
            && self.shown_render.as_deref() != Some(rendered.url.as_str())
        {
            self.shown_render = Some(rendered.url);
            self.chosen = Mode::Render;
        }
        cx.notify();
    }

    /// Asks the server for the timeline's clips.
    pub fn load(&mut self, cx: &mut Context<Self>) {
        let (project, target) = self.current(cx);
        self.asked = Some((project.clone(), target.clone()));
        self.asks += 1;
        let ask = self.asks;
        self.load_error = None;
        let Some(server) = self.state.read(cx).server.clone() else {
            self.loading = false;
            return;
        };
        self.loading = true;
        let query: Vec<String> = [
            ("project", project),
            ("id", target.id),
            ("commit", target.commit),
        ]
        .into_iter()
        .filter_map(|(key, value)| Some(format!("{key}={}", encode_component(&value?))))
        .collect();
        let path = format!("/api/timeline?{}", query.join("&"));
        cx.spawn(async move |this, cx| {
            let answer = cx
                .background_spawn(async move { fetch_timeline(&server, &path) })
                .await;
            let _ = this.update(cx, |this, cx| {
                if this.asks != ask {
                    return;
                }
                this.loading = false;
                match answer {
                    Ok(timeline) => this.loaded(timeline, cx),
                    Err(error) => this.load_error = Some(error),
                }
                cx.notify();
            });
        })
        .detach();
    }

    /// A timeline arrived. Another timeline (or version) starts at the beginning, cut preview.
    pub fn loaded(&mut self, timeline: PlayableTimeline, cx: &mut Context<Self>) {
        let same = self
            .timeline
            .as_ref()
            .is_some_and(|known| known.id == timeline.id && known.commit == timeline.commit);
        if !same {
            self.player.pause();
            self.player.seek(0.);
            self.chosen = Mode::Cut;
            self.confirm_apply = false;
        }
        self.timeline = Some(timeline);
        self.loading = false;
        self.load_error = None;
        cx.notify();
    }

    /// The render of the timeline in view, when there is one.
    fn rendered(&self, cx: &App) -> Option<Rendered> {
        let timeline = self.timeline.as_ref()?;
        match &self.state.read(cx).app.as_ref()?.meta.preview {
            PreviewState::Ready {
                timeline: rendered,
                url,
                outdated,
                ..
            } if rendered.id == timeline.id => Some(Rendered {
                url: url.clone(),
                outdated: *outdated,
            }),
            _ => None,
        }
    }

    /// How long the render at `url` runs: what ffmpeg says, the timeline's length until then.
    fn render_duration(&mut self, url: &str, fallback: f64, cx: &mut Context<Self>) -> f64 {
        if let Some((known, length)) = &self.render_length
            && known == url
        {
            return length.unwrap_or(fallback);
        }
        self.render_length = Some((url.to_string(), None));
        if let Some(server) = self.state.read(cx).server.clone() {
            let path = url.to_string();
            cx.spawn(async move |this, cx| {
                let full = server.url(&path);
                let probe = server.clone();
                let length = cx
                    .background_spawn(async move { ffmpeg::duration(&probe, &full) })
                    .await;
                let _ = this.update(cx, |this, cx| {
                    if let Some((known, unknown)) = &mut this.render_length
                        && *known == path
                    {
                        *unknown = length;
                        cx.notify();
                    }
                });
            })
            .detach();
        }
        fallback
    }

    fn seek(&mut self, time: f64) {
        self.player.seek(time);
    }

    fn toggle_play(&mut self) {
        if self.player.playing() {
            self.player.pause();
        } else {
            if self.player.time() >= self.player.duration() - 0.01 {
                self.player.seek(0.);
            }
            self.player.play();
        }
    }

    fn cut(&mut self, direction: i8) {
        if let Some(timeline) = &self.timeline {
            let time = nearest_cut(timeline, self.player.time(), direction);
            self.seek(time);
        }
    }

    fn toggle_fullscreen(&mut self, window: &mut Window) {
        self.fullscreen = !self.fullscreen;
        if window.is_fullscreen() != self.fullscreen {
            window.toggle_fullscreen();
        }
    }

    /// Where the playhead is, as feedback names it: the time, and the frame for the agent's tools.
    fn comment(&mut self, cx: &mut Context<Self>) {
        let Some(timeline) = &self.timeline else {
            return;
        };
        self.player.pause();
        let time = self.player.time();
        let whole = time.floor() as u64;
        let place = format!(
            "At {}:{:02} (frame {}) in \"{}\"",
            whole / 60,
            whole % 60,
            (time * timeline.fps).round(),
            timeline.name
        );
        cx.emit(PreviewEvent::Comment(place));
    }

    fn control(&mut self, control: Control, window: &mut Window, cx: &mut Context<Self>) {
        match control {
            Control::Seek(time) => self.seek(time),
            Control::Hover(hover) => self.hover = hover,
            Control::Drag(drag) => self.drag = drag,
            Control::TogglePlay => self.toggle_play(),
            Control::Cut(direction) => self.cut(direction),
            Control::ToggleMute => self.muted = !self.muted,
            Control::Volume(volume) => {
                self.volume = volume;
                self.muted = volume == 0.;
            }
            Control::ToggleFullscreen => self.toggle_fullscreen(window),
        }
        cx.notify();
    }

    fn on_key(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(timeline) = &self.timeline else {
            return;
        };
        let frame = 1. / timeline.fps;
        let time = self.player.time();
        let shift = event.keystroke.modifiers.shift;
        let step = if shift { 1. } else { frame };
        match event.keystroke.key.as_str() {
            "space" | "k" => self.toggle_play(),
            "left" => self.seek(time - step),
            "right" => self.seek(time + step),
            "up" => self.cut(-1),
            "down" => self.cut(1),
            "home" => self.seek(0.),
            "end" => self.seek(self.player.duration()),
            "f" => self.toggle_fullscreen(window),
            "escape" if self.fullscreen => self.toggle_fullscreen(window),
            "m" => self.muted = !self.muted,
            _ => return,
        }
        cx.stop_propagation();
        cx.notify();
    }

    fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.player.stop(window, cx);
        if self.fullscreen {
            self.toggle_fullscreen(window);
        }
        cx.emit(PreviewEvent::Close);
    }

    /// Lets go of the frames before the panel goes (the workspace closing it).
    pub fn release(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.player.stop(window, cx);
        if self.fullscreen {
            self.toggle_fullscreen(window);
        }
    }
}

/// How long the server gets to read the timeline out of Resolve.
const TIMELINE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// GET /api/timeline: the timeline, or the server's `{ error }`.
fn fetch_timeline(server: &ServerAddress, path: &str) -> Result<PlayableTimeline, String> {
    let body = http::get_any(server, path, TIMELINE_TIMEOUT)?
        .text()
        .map_err(|error| error.to_string())?;
    parse_timeline(&body)
}

fn parse_timeline(body: &str) -> Result<PlayableTimeline, String> {
    let value: serde_json::Value = serde_json::from_str(body).map_err(|error| error.to_string())?;
    if let Some(error) = value.get("error").and_then(|error| error.as_str()) {
        return Err(error.to_string());
    }
    serde_json::from_value(value).map_err(|error| error.to_string())
}

/// A JavaScript number as text: 42, or 42.5.
fn js_number(value: f64) -> String {
    if value.fract() == 0. {
        format!("{value:.0}")
    } else {
        value.to_string()
    }
}

/// Text with medium-weight parts, as the Svelte's inline `<span class="font-medium …">`s.
fn emphasized(parts: &[(&str, bool)], color: gpui::Hsla) -> StyledText {
    let mut text = String::new();
    let mut highlights: Vec<(Range<usize>, HighlightStyle)> = Vec::new();
    for (part, strong) in parts {
        let start = text.len();
        text.push_str(part);
        if *strong {
            highlights.push((
                start..text.len(),
                HighlightStyle {
                    color: Some(color),
                    font_weight: Some(FontWeight::MEDIUM),
                    ..Default::default()
                },
            ));
        }
    }
    StyledText::new(text).with_highlights(highlights)
}

fn tooltip(
    text: impl Into<SharedString>,
) -> impl Fn(&mut Window, &mut App) -> gpui::AnyView + 'static {
    let text = text.into();
    move |window, cx| Tooltip::new(text.clone()).build(window, cx)
}

impl Render for PreviewPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let (preview, resolve_connected, busy) = {
            let state = self.state.read(cx);
            let app = state.app.as_ref();
            (
                app.map_or(PreviewState::Idle, |app| app.meta.preview.clone()),
                app.is_some_and(|app| matches!(app.meta.resolve, ResolveStatus::Connected { .. })),
                state.resolve_busy(),
            )
        };
        let target = self.current(cx).1;
        let rendered = self.rendered(cx);
        // Rendered only while a render of this timeline exists: a new render deletes the last one.
        let source = self.source(cx);
        let replaces: Option<TimelineRef> = if target.commit.is_none() {
            self.timeline
                .as_ref()
                .and_then(|timeline| timeline.replaces.clone())
        } else {
            None
        };
        let render_blocked = if target.commit.is_some() {
            Some(
                "Only the timeline as it is in Resolve now can be rendered. Restore this version first.",
            )
        } else if !resolve_connected {
            Some("Connect Resolve to render.")
        } else if busy {
            Some("Wait for the agent to finish.")
        } else if self.timeline.is_none() {
            Some("Load a timeline first.")
        } else {
            None
        };

        // The player follows what is chosen, at the stage's size.
        let program = match (self.timeline.clone(), &rendered) {
            (Some(timeline), Some(rendered)) if source == Mode::Render => {
                let duration = self.render_duration(&rendered.url, timeline.duration, cx);
                Some(Program::render(&rendered.url, &timeline, duration))
            }
            (Some(timeline), _) => Some(Program::cut(timeline)),
            _ => None,
        };
        self.player.set_stage(self.stage.get());
        self.player.set_volume(self.muted, self.volume);
        self.player.set_program(program, window, cx);
        let pictures = self.player.tick(window, cx);
        let mut thumbnails = HashMap::new();
        if let Some(timeline) = &self.timeline {
            let scale = window.scale_factor();
            let size = ((64. * scale) as u32, (36. * scale) as u32);
            for clip in &timeline.video {
                if clip.track == 1
                    && clip.kind == linker_edit_protocol::MediaKind::Image
                    && let Some(media) = &clip.media
                    && let Some(image) = self.player.thumbnail(media, size, window, cx)
                {
                    thumbnails.insert(media.clone(), image);
                }
            }
        }
        // Draw again when the next frame is due (not at the display's rate: the video's is lower).
        self.redraw = self.player.next_draw().map(|wait| {
            cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                let _ = this.update(cx, |_, cx| cx.notify());
            })
        });
        let time = self.player.time();
        let playing = self.player.playing();
        let duration = self.player.duration();

        let header = panel_header(Panel::Preview, "Preview", window, &theme).child(close_button(
            "Close preview",
            &theme,
            cx.listener(|this, _, window, cx| this.close(window, cx)),
        ));

        let link = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .cursor_pointer()
                .hover(|style| style.text_color(theme.foreground).underline())
                .child(label)
                .test_support()
        };
        let subtitle = div()
            .flex()
            .items_center()
            .gap_1()
            .text_token(TextSize::X2s)
            .text_color(theme.muted_foreground)
            .map(|this| match &target.commit {
                Some(commit) => this
                    .child(Icon::new(IconName::History).size(px(12.)))
                    .child("Version from save")
                    .child(
                        div()
                            .font_family(theme.mono_font_family)
                            .child(commit.clone()),
                    )
                    .child("·")
                    .child(
                        link("show-current", "show it as it is now").on_click(cx.listener(
                            |this, _, _, cx| {
                                let target = PreviewTarget {
                                    id: this.current(cx).1.id,
                                    commit: None,
                                };
                                cx.emit(PreviewEvent::Target(target));
                            },
                        )),
                    ),
                None => this.child("As it is in Resolve now ·").child(
                    link("reload", "reload").on_click(cx.listener(|this, _, _, cx| this.load(cx))),
                ),
            });
        let name = match &self.timeline {
            Some(timeline) => timeline.name.clone(),
            None if self.loading => "Loading timeline…".into(),
            None => "No timeline".into(),
        };
        let action = match &preview {
            PreviewState::Rendering { percent, .. } => div()
                .id("rendering")
                .flex()
                .flex_shrink_0()
                .h_8()
                .items_center()
                .gap_2()
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.card)
                .pl_2p5()
                .pr_1()
                .text_token(TextSize::Xs)
                .child(
                    Spinner::new()
                        .icon(Icon::new(IconName::LoaderCircle))
                        .ease(|delta| delta)
                        .color(theme.primary)
                        .with_size(px(14.)),
                )
                .child(format!("Rendering {}%", js_number(*percent)))
                .child(
                    div()
                        .id("stop-render")
                        .flex()
                        .size_6()
                        .items_center()
                        .justify_center()
                        .rounded(theme.radius.md)
                        .text_color(theme.muted_foreground)
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
                        .tooltip(tooltip("Stop rendering"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.state.read(cx).send(ClientMessage::StopPreview)
                        }))
                        .child(Icon::new(IconName::X).size(px(14.)))
                        .test_support(),
                )
                .test_support(),
            _ => {
                let (icon, label) = if rendered.is_some() {
                    (IconName::RefreshCw, "Render again")
                } else {
                    (IconName::Clapperboard, "Full render")
                };
                let blocked = render_blocked.is_some();
                div()
                    .id("render")
                    .flex()
                    .flex_shrink_0()
                    .h_8()
                    .items_center()
                    .gap_1p5()
                    .rounded(theme.radius.lg)
                    .bg(theme.primary)
                    .px_3()
                    .text_token(TextSize::Xs)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.primary_foreground)
                    .when(blocked, |this| this.opacity(0.4))
                    .when(!blocked, |this| {
                        this.cursor_pointer()
                            .hover(|style| style.opacity(0.9))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(timeline) = &this.timeline {
                                    this.state.read(cx).send(ClientMessage::RenderPreview {
                                        timeline_id: Some(timeline.id.clone()),
                                    });
                                }
                            }))
                    })
                    .tooltip(tooltip(render_blocked.unwrap_or(
                        "Have Resolve render the timeline with color, effects and transitions",
                    )))
                    .child(Icon::new(icon).size(px(14.)))
                    .child(label)
                    .test_support()
            }
        };
        let title_row = div()
            .flex()
            .min_w_0()
            .items_center()
            .gap_2()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(
                        div()
                            .id("timeline-name")
                            .truncate()
                            .text_token(TextSize::Sm)
                            .font_weight(FontWeight::MEDIUM)
                            .child(name)
                            .test_support(),
                    )
                    .child(subtitle),
            )
            .child(action);

        let apply = replaces.filter(|_| self.timeline.is_some()).map(|replaces| {
            let confirm = self.confirm_apply;
            let strong = theme.foreground;
            let text = if confirm {
                emphasized(
                    &[
                        ("This takes the name ", false),
                        (&format!("\"{}\"", replaces.name), true),
                        (
                            &format!(
                                ", and the current one stays as \"{} (before AI …)\". The project is backed up first.",
                                replaces.name
                            ),
                            false,
                        ),
                    ],
                    strong,
                )
            } else {
                emphasized(
                    &[
                        ("A new version of ", false),
                        (&format!("\"{}\"", replaces.name), true),
                        (". Happy with it?", false),
                    ],
                    strong,
                )
            };
            let title = if busy {
                "Wait for the agent to finish.".to_string()
            } else {
                format!("Make this the main \"{}\" timeline", replaces.name)
            };
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.card)
                .px_3()
                .py_2()
                .text_token(TextSize::Xs)
                .child(div().min_w_0().flex_1().text_color(theme.muted_foreground).child(text))
                .when(confirm, |this| {
                    this.child(
                        div()
                            .id("cancel-apply")
                            .flex()
                            .h_7()
                            .items_center()
                            .rounded(theme.radius.md)
                            .px_2()
                            .text_color(theme.muted_foreground)
                            .cursor_pointer()
                            .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.confirm_apply = false;
                                cx.notify();
                            }))
                            .child("Cancel")
                            .test_support(),
                    )
                })
                .child(
                    div()
                        .id("apply")
                        .flex()
                        .flex_shrink_0()
                        .h_7()
                        .items_center()
                        .gap_1p5()
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(theme.border)
                        .px_2p5()
                        .font_weight(FontWeight::MEDIUM)
                        .when(busy, |this| this.opacity(0.4))
                        .when(!busy, |this| {
                            this.cursor_pointer()
                                .hover(|style| style.bg(theme.accent))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if !this.confirm_apply {
                                        this.confirm_apply = true;
                                    } else {
                                        this.confirm_apply = false;
                                        let state = this.state.read(cx);
                                        if let (Some(timeline), Some(project_id)) =
                                            (&this.timeline, state.project_id())
                                        {
                                            state.send(ClientMessage::ApplyTimeline {
                                                project_id: project_id.to_string(),
                                                timeline_id: timeline.id.clone(),
                                            });
                                        }
                                    }
                                    cx.notify();
                                }))
                        })
                        .tooltip(tooltip(title))
                        .child(Icon::new(IconName::Check).size(px(14.)))
                        .child(if confirm {
                            format!("Replace \"{}\"", replaces.name)
                        } else {
                            "Use as main timeline".into()
                        })
                        .test_support(),
                )
        });

        let modes = [
            (
                Mode::Cut,
                "cut-preview",
                "Cut preview",
                "Instant, no color or effects",
                IconName::Scissors,
            ),
            (
                Mode::Render,
                "full-render",
                "Full render",
                "Exactly as Resolve plays it",
                IconName::Clapperboard,
            ),
        ]
        .map(|(mode, id, label, detail, icon)| {
            let unavailable = mode == Mode::Render && rendered.is_none();
            let pressed = source == mode;
            div()
                .id(id)
                .flex()
                .flex_1()
                .min_w_0()
                .items_start()
                .gap_2()
                .rounded(theme.radius.md)
                .px_2p5()
                .py_1p5()
                .map(|this| {
                    if pressed {
                        this.bg(theme.accent).text_color(theme.foreground)
                    } else {
                        this.text_color(theme.muted_foreground)
                    }
                })
                .when(unavailable, |this| {
                    this.opacity(0.5).tooltip(tooltip(
                        "Use Full render to watch the timeline exactly as Resolve plays it",
                    ))
                })
                .when(!unavailable, |this| {
                    this.cursor_pointer()
                        .when(!pressed, |this| {
                            this.hover(|style| style.text_color(theme.foreground))
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.chosen = mode;
                            cx.notify();
                        }))
                })
                .child(
                    div()
                        .mt_0p5()
                        .flex_shrink_0()
                        .child(Icon::new(icon).size(px(14.))),
                )
                .child(
                    div()
                        .min_w_0()
                        .child(
                            div()
                                .text_token(TextSize::Xs)
                                .font_weight(FontWeight::MEDIUM)
                                .child(label),
                        )
                        .child(
                            div()
                                .truncate()
                                .text_token(TextSize::X2s)
                                .text_color(theme.muted_foreground)
                                .child(if unavailable {
                                    "Not rendered yet"
                                } else {
                                    detail
                                }),
                        ),
                )
                .test_support()
        });
        let modes = div()
            .flex()
            .gap_0p5()
            .rounded(theme.radius.lg)
            .border_1()
            .border_color(theme.border)
            .bg(theme.card)
            .p_0p5()
            .children(modes);

        // The stage: the picture, then the controls under it.
        let stage_size = self.stage.clone();
        let measure = canvas(
            move |bounds, window, _| {
                let scale = window.scale_factor();
                let size = (
                    (f32::from(bounds.size.width) * scale).round() as u32,
                    (f32::from(bounds.size.height) * scale).round() as u32,
                );
                if stage_size.replace(size) != size {
                    window.refresh();
                }
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let picture: AnyElement = if self.timeline.is_some() {
            let corner = if self.fullscreen {
                px(0.)
            } else {
                theme.radius.xl
            };
            TimelinePlayer::new(pictures, corner).into_any_element()
        } else {
            div()
                .flex()
                .size_full()
                .items_center()
                .justify_center()
                .text_token(TextSize::Xs)
                .text_color(white(0.6))
                .map(|this| {
                    if self.loading {
                        this.child(
                            Spinner::new()
                                .icon(Icon::new(IconName::LoaderCircle))
                                .ease(|delta| delta)
                                .with_size(px(20.)),
                        )
                    } else if let Some(error) = &self.load_error {
                        this.child(
                            div()
                                .id("load-error")
                                .flex()
                                .items_start()
                                .gap_2()
                                .px_6()
                                .text_center()
                                .child(
                                    div()
                                        .mt(px(1.))
                                        .flex_shrink_0()
                                        .child(Icon::new(IconName::CircleAlert).size(px(14.))),
                                )
                                .child(error.clone())
                                .test_support(),
                        )
                    } else {
                        this.child("Open a timeline in Resolve to preview it.")
                    }
                })
                .into_any_element()
        };
        let aspect = self.timeline.as_ref().map_or(16. / 9., |timeline| {
            timeline.width as f32 / timeline.height.max(1) as f32
        });
        let badge = self.timeline.as_ref().map(|_| {
            let (icon, label) = if source == Mode::Render {
                (IconName::Clapperboard, "FULL RENDER")
            } else {
                (IconName::Scissors, "CUT PREVIEW")
            };
            div()
                .id("stage-badge")
                .absolute()
                .top_2()
                .left_2()
                .flex()
                .items_center()
                .gap_1()
                .rounded(theme.radius.md)
                .bg(black(0.65))
                .px_1p5()
                .py_0p5()
                .text_size(px(10.))
                .line_height(px(15.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(gpui::white())
                .child(Icon::new(icon).size(px(12.)))
                .child(label)
                .test_support()
        });
        let fullscreen = self.fullscreen;
        let on_control: OnControl = {
            let this = cx.entity().downgrade();
            Rc::new(move |control, window, cx| {
                let _ = this.update(cx, |this, cx| this.control(control, window, cx));
            })
        };
        let controls = PlayerControls {
            timeline: self.timeline.clone(),
            thumbnails,
            duration,
            time,
            playing,
            muted: self.muted,
            volume: self.volume,
            fullscreen,
            hover: self.hover,
            drag: self.drag,
            on: on_control,
        };
        let stage = div()
            .id("preview-stage")
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .overflow_hidden()
            .bg(rgb(0x0a0a0a))
            .when(!fullscreen, |this| this.rounded(theme.radius.xl))
            .when(fullscreen, |this| this.size_full().justify_center())
            .on_key_down(cx.listener(Self::on_key))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _, window, cx| {
                    window.focus(&this.focus, cx);
                }),
            )
            .child(
                div()
                    .id("stage-picture")
                    .relative()
                    .w_full()
                    .map(|this| {
                        if fullscreen {
                            this.flex_1().min_h_0()
                        } else {
                            this.aspect_ratio(aspect)
                        }
                    })
                    .when(self.timeline.is_some(), |this| {
                        this.cursor_pointer().on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.toggle_play();
                                cx.notify();
                            },
                        ))
                    })
                    .child(measure)
                    .child(picture)
                    .children(badge)
                    .test_support(),
            )
            .child(controls)
            .test_support();
        let stage: AnyElement = if fullscreen {
            let viewport = window.viewport_size();
            deferred(
                anchored()
                    .position(point(px(0.), px(0.)))
                    .child(div().w(viewport.width).h(viewport.height).child(stage)),
            )
            .with_priority(2)
            .into_any_element()
        } else {
            stage.into_any_element()
        };

        let feedback = (self.timeline.is_some() && target.commit.is_none()).then(|| {
            div()
                .id("feedback")
                .flex()
                .h_8()
                .items_center()
                .justify_center()
                .gap_1p5()
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.card)
                .text_token(TextSize::Xs)
                .font_weight(FontWeight::MEDIUM)
                .cursor_pointer()
                .hover(|style| style.bg(theme.accent))
                .tooltip(tooltip(
                    "Starts a message about this moment, so the agent knows where you mean",
                ))
                .on_click(cx.listener(|this, _, _, cx| this.comment(cx)))
                .child(Icon::new(IconName::MessageSquarePlus).size(px(14.)))
                .child("Give feedback at this moment")
                .test_support()
        });
        let outdated = (source == Mode::Render && rendered.as_ref().is_some_and(|r| r.outdated)).then(|| {
            div()
                .id("render-outdated")
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.warning.opacity(0.3))
                .bg(theme.warning.opacity(0.1))
                .px_3()
                .py_2()
                .text_token(TextSize::Xs)
                .text_color(theme.warning_foreground)
                .child("Resolve changed after this render. Cut preview shows the latest cut; render again for the exact picture.")
                .test_support()
        });
        let error = match &preview {
            PreviewState::Error { message } => Some(
                div()
                    .id("render-error")
                    .flex()
                    .items_start()
                    .gap_2()
                    .rounded(theme.radius.lg)
                    .border_1()
                    .border_color(theme.destructive.opacity(0.25))
                    .bg(theme.destructive.opacity(0.08))
                    .px_3()
                    .py_2()
                    .text_token(TextSize::Xs)
                    .text_color(theme.destructive)
                    .child(
                        div()
                            .mt(px(1.))
                            .flex_shrink_0()
                            .child(Icon::new(IconName::CircleAlert).size(px(14.))),
                    )
                    .child(message.clone())
                    .test_support(),
            ),
            _ => None,
        };
        let strong = theme.foreground.opacity(0.8);
        let note = div()
            .mt_auto()
            .text_size(px(11.))
            .line_height(px(11. * 1.625))
            .text_color(theme.muted_foreground)
            .child(emphasized(
                &[
                    ("Cut preview", true),
                    (" plays your media straight from disk, right away and for any saved version: the cuts, zoom, position and opacity, but no color, effects, transitions or titles. ", false),
                    ("Full render", true),
                    (" has Resolve render the exact picture as a small MP4; Resolve switches to the timeline while it renders and puts everything back after. Keys: Space play, ← → frame, ↑ ↓ cut, F full screen.", false),
                ],
                strong,
            ));

        panel_frame(Panel::Preview, &theme)
            .child(header)
            .child(
                div()
                    .id("preview-body")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_3()
                    .overflow_y_scroll()
                    .p_4()
                    .child(title_row)
                    .children(apply)
                    .child(modes)
                    .child(stage)
                    .children(feedback)
                    .children(outdated)
                    .children(error)
                    .child(note),
            )
            .test_support()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::{Link, LinkEvent, ServerEnd, ServerMessage};
    use crate::workspace::Workspace;
    use gpui::{AnyWindowHandle, Bounds, TestAppContext, WindowBounds, WindowOptions, size};
    use gpui_kit::test::TestWindowExt;
    use linker_edit_protocol::{MediaKind, PlayableClip, PreviewTimeline};

    fn clip(name: &str, start: f64, end: f64, kind: MediaKind) -> PlayableClip {
        PlayableClip {
            name: name.into(),
            track: 1,
            start,
            end,
            offset: 0.,
            rate: 1.,
            media: (kind != MediaKind::Missing).then(|| name.into()),
            kind,
            look: Default::default(),
        }
    }

    /// A title nobody can play over [0, 2), a video until 5; an AI version of "Main cut".
    fn timeline() -> PlayableTimeline {
        PlayableTimeline {
            id: "tl1".into(),
            name: "AI - Cut".into(),
            commit: None,
            fps: 24.,
            width: 1920,
            height: 1080,
            duration: 5.,
            video: vec![
                clip("Title", 0., 2., MediaKind::Missing),
                clip("a.mov", 2., 5., MediaKind::Video),
            ],
            audio: vec![],
            markers: vec![],
            replaces: Some(TimelineRef {
                id: "main".into(),
                name: "Main cut".into(),
            }),
        }
    }

    fn connected() -> ResolveStatus {
        ResolveStatus::Connected {
            product: "DaVinci Resolve Studio".into(),
            version: "20".into(),
            project: None,
            project_id: None,
            database: None,
            timeline: None,
        }
    }

    /// The window as `run` opens it, Resolve connected, on a link the test drives.
    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<Workspace>, ServerEnd) {
        cx.update(gpui_kit::init);
        let (link, server) = Link::pair();
        let root = Rc::new(std::cell::RefCell::new(None));
        let kept = root.clone();
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
                let workspace = crate::new_workspace(state, window, cx);
                *kept.borrow_mut() = Some(workspace.clone());
                workspace
            })
            .unwrap()
            .0
        });
        let mut snapshot = crate::state::tests::snapshot();
        snapshot.meta.resolve = connected();
        server.events.unbounded_send(LinkEvent::Connected).unwrap();
        let message = ServerMessage::Snapshot {
            state: Box::new(snapshot),
        };
        server
            .events
            .unbounded_send(LinkEvent::Message(message))
            .unwrap();
        let workspace = root.borrow_mut().take().unwrap();
        (window, workspace, server)
    }

    fn draw<R>(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        cx.run_until_parked();
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx)
        })
        .unwrap()
    }

    fn set_preview(server: &ServerEnd, preview: PreviewState) {
        let mut meta = crate::state::tests::snapshot().meta;
        meta.resolve = connected();
        meta.preview = preview;
        let message = ServerMessage::Meta {
            meta: Box::new(meta),
        };
        server
            .events
            .unbounded_send(LinkEvent::Message(message))
            .unwrap();
    }

    fn sent(server: &mut ServerEnd) -> Vec<ClientMessage> {
        std::iter::from_fn(|| server.commands.try_recv().ok()).collect()
    }

    #[gpui::test]
    fn plays_a_timeline_renders_it_and_starts_feedback_at_the_playhead(cx: &mut TestAppContext) {
        let (window, workspace, mut server) = open(cx);
        draw(cx, window, |window, cx| window.click("preview", cx));
        let panel = workspace.read_with(cx, |workspace, _| workspace.preview().cloned().unwrap());
        // Without a server address there is nothing to load or decode; the timeline is handed
        // over as if /api/timeline had answered.
        panel.update(cx, |panel, cx| panel.loaded(timeline(), cx));
        let time = |cx: &mut TestAppContext| panel.read_with(cx, |panel, _| panel.player().time());
        let playing =
            |cx: &mut TestAppContext| panel.read_with(cx, |panel, _| panel.player().playing());

        draw(cx, window, |window, cx| {
            assert!(window.find("preview-stage").visible());
            // The title can't play: it says what it is instead. Both clips are on the scrubber.
            assert!(window.try_find(("unplayable", 0usize)).is_some());
            assert!(window.try_find(("scrubber-clip", 1usize)).is_some());
            window.click("next-cut", cx);
        });
        assert_eq!(time(cx), 2.);
        draw(cx, window, |window, cx| {
            assert!(window.try_find(("picture", 0usize)).is_some());
            window.click("play", cx);
        });
        assert!(playing(cx));
        draw(cx, window, |window, cx| window.click("play", cx));
        assert!(!playing(cx));
        // From wherever it stopped, the end; then back cut by cut.
        draw(cx, window, |window, cx| window.click("next-cut", cx));
        assert_eq!(time(cx), 5.);
        for cut in [2., 0.] {
            draw(cx, window, |window, cx| window.click("previous-cut", cx));
            assert_eq!(time(cx), cut);
        }
        draw(cx, window, |window, cx| window.click("next-cut", cx));

        // Full render asks Resolve; while it renders there is a stop button.
        draw(cx, window, |window, cx| window.click("render", cx));
        assert_eq!(
            sent(&mut server),
            [ClientMessage::RenderPreview {
                timeline_id: Some("tl1".into())
            }]
        );
        set_preview(
            &server,
            PreviewState::Rendering {
                timeline: None,
                percent: 40.,
            },
        );
        draw(cx, window, |window, cx| window.click("stop-render", cx));
        assert_eq!(sent(&mut server), [ClientMessage::StopPreview]);

        // A render that finishes takes over, at the same place; the cut preview is a click away.
        let ready = PreviewState::Ready {
            timeline: PreviewTimeline {
                id: "tl1".into(),
                name: "AI - Cut".into(),
            },
            url: "/preview/tl1.mp4".into(),
            rendered_at: "2026-11-18T10:00:00Z".into(),
            outdated: true,
        };
        set_preview(&server, ready);
        draw(cx, window, |window, _| {
            assert!(window.try_find("render-outdated").is_some());
        });
        assert_eq!(
            panel.read_with(cx, |panel, cx| panel.source(cx)),
            Mode::Render
        );
        assert_eq!(time(cx), 2.);
        draw(cx, window, |window, cx| window.click("cut-preview", cx));
        assert_eq!(panel.read_with(cx, |panel, cx| panel.source(cx)), Mode::Cut);

        // Feedback starts a message about this moment.
        draw(cx, window, |window, cx| window.click("feedback", cx));
        let draft = workspace.read_with(cx, |workspace, cx| {
            workspace.chat().read(cx).composer().read(cx).draft(cx)
        });
        assert_eq!(draft, "At 0:02 (frame 48) in \"AI - Cut\": ");

        // Making it the main timeline takes a second click.
        draw(cx, window, |window, cx| window.click("apply", cx));
        assert!(sent(&mut server).is_empty());
        draw(cx, window, |window, cx| window.click("apply", cx));
        let project_id = crate::state::tests::snapshot().meta.project_id.unwrap();
        assert_eq!(
            sent(&mut server),
            [ClientMessage::ApplyTimeline {
                project_id,
                timeline_id: "tl1".into()
            }]
        );

        draw(cx, window, |window, cx| window.click("close-panel", cx));
        draw(cx, window, |window, _| {
            assert!(window.try_find("preview-panel").is_none())
        });
        assert!(workspace.read_with(cx, |workspace, _| workspace.preview().is_none()));
    }

    #[test]
    fn reads_the_timeline_or_the_servers_error() {
        assert_eq!(
            parse_timeline(r#"{ "error": "That timeline isn't in Resolve or in that save." }"#),
            Err("That timeline isn't in Resolve or in that save.".into())
        );
        let timeline = parse_timeline(
            r#"{ "id": "t", "name": "AI - Cut", "commit": null, "fps": 24, "width": 1920, "height": 1080,
                 "duration": 10, "video": [], "audio": [], "markers": [] }"#,
        )
        .unwrap();
        assert_eq!(timeline.name, "AI - Cut");
        assert_eq!(js_number(42.), "42");
        assert_eq!(js_number(42.5), "42.5");
    }
}
