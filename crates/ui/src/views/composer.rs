//! Composer.svelte with ProviderPicker.svelte and ContextMeter.svelte: the message box, then the
//! agent, the approval mode, how full the context is, compacting, and send or stop.
//!
//! The draft is the window's own business: one per thread, like +page.svelte's `drafts`, so
//! switching threads never sends a draft to the wrong one. The one in view lives in the input.
//! Enter sends, Shift+Enter starts a new line.

use std::collections::HashMap;
use std::f32::consts::PI;
use std::time::Duration;

use chrono::{DateTime, Utc};
use gpui::{
    Anchor, App, AppContext, Context, Entity, EventEmitter, FontWeight, Hsla, InteractiveElement,
    IntoElement, ParentElement, PathBuilder, Render, SharedString, StatefulInteractiveElement,
    Styled, Subscription, Task, Window, canvas, div, point, prelude::FluentBuilder, px, relative,
};
use gpui_kit::TestSupportExt;
use gpui_kit::base::Popover;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{Icon, Sizable, Size};
use linker_edit_protocol::{ApprovalMode, ClientMessage, ContextUsage, Provider};

use crate::context::{cache_expiry, compact_suggested, format_tokens, group_thousands};
use crate::icons::{IconName, Logo};
use crate::labels::provider_label;
use crate::state::{AppEvent, AppState};
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::approval_card::composer_shadow;

/// Shown when Claude's cache has expired on a big chat: "compact" summarizes before sending.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompactChip {
    Compact,
    Full,
}

/// Something the chat follows: a message went, so it scrolls to the end again.
pub enum ComposerEvent {
    Sent,
}

pub struct Composer {
    state: Entity<AppState>,
    input: Entity<TextareaState>,
    /// The drafts of threads not in view, by thread id.
    drafts: HashMap<String, String>,
    /// The thread whose draft is in the input; "" before there is one.
    thread: String,
    provider: Option<Provider>,
    /// The cache expiry the user switched to sending the full chat for.
    full_for: Option<String>,
    /// The time the compact suggestion was last checked: it wakes once at the cache's expiry.
    now: DateTime<Utc>,
    expiry: Option<(String, Task<()>)>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<ComposerEvent> for Composer {}

impl Composer {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input = cx.new(|cx| {
            TextareaState::new(window, cx)
                // min-h-16 and max-h-64 of the box, with its padding: one line up to nine.
                .auto_grow(1, 9)
                .submit_on_enter(true)
                .placeholder("Ask Claude to edit…")
        });
        let subscriptions =
            vec![
                cx.subscribe_in(&input, window, |this, _, event: &InputEvent, window, cx| {
                    match event {
                        InputEvent::PressEnter { shift: false, .. } => this.send(window, cx),
                        InputEvent::Change => cx.notify(),
                        _ => {}
                    }
                }),
                cx.observe_in(&state, window, |this, _, window, cx| this.sync(window, cx)),
                cx.subscribe_in(&state, window, |this, _, event: &AppEvent, window, cx| {
                    let AppEvent::Refused { thread_id, text } = event;
                    this.refused(thread_id, text, window, cx);
                }),
            ];
        let mut composer = Composer {
            state,
            input,
            drafts: HashMap::new(),
            thread: String::new(),
            provider: None,
            full_for: None,
            now: Utc::now(),
            expiry: None,
            _subscriptions: subscriptions,
        };
        composer.sync(window, cx);
        composer
    }

    /// What is typed in the thread in view.
    pub fn draft(&self, cx: &App) -> String {
        self.input.read(cx).value().to_string()
    }

    pub fn set_draft(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_value(text.to_string(), window, cx)
        });
        cx.notify();
    }

    pub fn focus(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| input.focus(window, cx));
    }

    /// Follows the thread in view, its agent, and its cache's expiry.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let thread = state.thread_id().unwrap_or_default().to_string();
        let app = state.app.as_ref();
        let provider = app.map(|app| app.meta.provider);
        let expires = app
            .and_then(|app| app.meta.context.as_ref())
            .and_then(|context| context.cache_expires_at.clone());
        let expiry = cache_expiry(app.and_then(|app| app.meta.context.as_ref()));

        if thread != self.thread {
            let draft = self.draft(cx);
            if draft.trim().is_empty() {
                self.drafts.remove(&self.thread);
            } else {
                self.drafts.insert(self.thread.clone(), draft);
            }
            let next = self.drafts.remove(&thread).unwrap_or_default();
            self.input
                .update(cx, |input, cx| input.set_value(next, window, cx));
            self.thread = thread;
        }
        if provider != self.provider {
            self.provider = provider;
            let placeholder = format!(
                "Ask {} to edit…",
                provider_label(provider.unwrap_or(Provider::Claude))
            );
            self.input.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        // Wakes once when Claude's prompt cache expires, to offer compacting; no ticking clock.
        if expires.as_ref() != self.expiry.as_ref().map(|(at, _)| at) {
            self.expiry = None;
            if let (Some(at), Some(expiry)) = (expires, expiry) {
                let wait = (expiry - Utc::now()).to_std().unwrap_or_default();
                if wait.is_zero() {
                    self.now = Utc::now();
                } else {
                    let task = cx.spawn(async move |this, cx| {
                        cx.background_executor()
                            .timer(wait + Duration::from_secs(1))
                            .await;
                        let _ = this.update(cx, |this, cx| {
                            this.now = Utc::now();
                            cx.notify();
                        });
                    });
                    self.expiry = Some((at, task));
                }
            }
        }
        cx.notify();
    }

    fn refused(
        &mut self,
        thread_id: &str,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if thread_id == self.thread {
            if self.draft(cx).trim().is_empty() {
                self.set_draft(text, window, cx);
            }
        } else if self
            .drafts
            .get(thread_id)
            .is_none_or(|draft| draft.trim().is_empty())
        {
            self.drafts.insert(thread_id.to_string(), text.to_string());
        }
    }

    fn compact_chip(&self, cx: &App) -> Option<CompactChip> {
        let app = self.state.read(cx).app.as_ref()?;
        let context = app.meta.context.as_ref();
        if !compact_suggested(app.meta.provider, context, self.now) {
            return None;
        }
        Some(
            if self.full_for.is_some() && self.full_for == context?.cache_expires_at {
                CompactChip::Full
            } else {
                CompactChip::Compact
            },
        )
    }

    /// Sends the draft, when it can go.
    pub fn send(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.draft(cx);
        let chip = self.compact_chip(cx);
        let state = self.state.read(cx);
        if !state.can_send(&draft) {
            return;
        }
        state.send(ClientMessage::Send {
            text: draft.trim().to_string(),
            compact_first: Some(chip == Some(CompactChip::Compact)),
        });
        self.set_draft("", window, cx);
        cx.emit(ComposerEvent::Sent);
    }

    fn send_to_server(&self, message: ClientMessage, cx: &App) {
        self.state.read(cx).send(message);
    }
}

impl Render for Composer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let draft = self.draft(cx);
        let chip = self.compact_chip(cx);
        let state = self.state.read(cx);
        let can_send = state.can_send(&draft);
        let Some(app) = state.app.as_ref() else {
            return div().into_any_element();
        };
        let provider = app.meta.provider;
        let asking = app.meta.approval_mode == ApprovalMode::Ask;
        let busy = app.meta.busy;
        let context = app.meta.context.clone();

        let mode_tip: SharedString = if asking {
            if provider == Provider::Codex {
                "Supervised: Resolve changes wait for your OK. Codex's shell commands and file edits run sandboxed in the project folder. Click for full access."
            } else {
                "Supervised: changes and scripts wait for your OK. Click for full access."
            }
        } else {
            "Full access: changes run without asking. Only AI timelines change, nothing is deleted, and the edit history keeps every version. Click to approve changes first."
        }
        .into();
        let approval_mode = div()
            .id("approval-mode")
            .flex()
            .items_center()
            .gap_1p5()
            .h_7()
            .px_2()
            .rounded(theme.radius.md)
            .text_token(TextSize::Sm)
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
            .tooltip(move |window, cx| Tooltip::new(mode_tip.clone()).build(window, cx))
            .on_click(cx.listener(move |this, _, _, cx| {
                let mode = if asking {
                    ApprovalMode::Auto
                } else {
                    ApprovalMode::Ask
                };
                this.send_to_server(ClientMessage::SetApprovalMode { mode }, cx);
            }))
            .child(
                Icon::new(if asking {
                    IconName::ShieldCheck
                } else {
                    IconName::ShieldOff
                })
                .size(px(14.)),
            )
            .child(if asking { "Supervised" } else { "Full access" })
            .test_support();

        let chip_element = match (chip, &context) {
            (Some(chip), Some(context)) if !busy => {
                let tip: SharedString = match chip {
                    CompactChip::Compact => format!(
                        "Claude's cache has expired. Sending summarizes the chat ({} tokens) first, so it isn't all reread. Click to send the full chat instead.",
                        format_tokens(context.used_tokens)
                    )
                    .into(),
                    CompactChip::Full => "Sends the full chat. Click to summarize it first.".into(),
                };
                let expires = context.cache_expires_at.clone();
                Some(
                    div()
                        .id("compact-chip")
                        .flex()
                        .items_center()
                        .h_7()
                        .px_2p5()
                        .rounded_full()
                        .border_1()
                        .border_color(theme.border)
                        .text_token(TextSize::Xs)
                        .text_color(if chip == CompactChip::Compact {
                            theme.foreground
                        } else {
                            theme.muted_foreground
                        })
                        .cursor_pointer()
                        .hover(|style| style.bg(theme.accent))
                        .tooltip(move |window, cx| Tooltip::new(tip.clone()).build(window, cx))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.full_for = match chip {
                                CompactChip::Compact => expires.clone(),
                                CompactChip::Full => None,
                            };
                            cx.notify();
                        }))
                        .child(match chip {
                            CompactChip::Compact => "Compact",
                            CompactChip::Full => "Full",
                        })
                        .test_support(),
                )
            }
            _ => None,
        };

        let action = if busy {
            div()
                .id("stop")
                .flex()
                .items_center()
                .justify_center()
                .size_8()
                .rounded_full()
                .bg(theme.foreground)
                .cursor_pointer()
                .hover(|style| style.opacity(0.85))
                .on_click(
                    cx.listener(|this, _, _, cx| this.send_to_server(ClientMessage::Stop, cx)),
                )
                .child(div().size_3().rounded(px(2.)).bg(theme.background))
                .test_support()
        } else {
            div()
                .id("send")
                .flex()
                .items_center()
                .justify_center()
                .size_8()
                .rounded_full()
                .bg(theme.primary)
                .text_color(theme.primary_foreground)
                .map(|this| {
                    if can_send {
                        this.cursor_pointer()
                            .hover(|style| style.opacity(0.9))
                            .on_click(cx.listener(|this, _, window, cx| this.send(window, cx)))
                    } else {
                        this.opacity(0.3)
                    }
                })
                .child(Icon::new(IconName::ArrowUp).size(px(16.)))
                .test_support()
        };

        div()
            .id("composer")
            .relative()
            .flex()
            .flex_col()
            .rounded(theme.radius.xl2)
            .border_1()
            .border_color(theme.composer_outline)
            .bg(theme.card)
            .shadow(composer_shadow(&theme))
            .child(
                // px-4 pt-3.5 pb-1, less the 4px the extra-small input keeps at its sides.
                div().min_h_16().px_3().pt(px(14.)).pb_1().child(
                    Textarea::new(&self.input)
                        .appearance(false)
                        .with_size(Size::XSmall)
                        .text_size(TextSize::Composer.size())
                        .line_height(relative(1.625)),
                ),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .px_2()
                    .pb_2()
                    .child(provider_picker(self.state.clone(), provider, &theme))
                    .child(div().mx_0p5().h_4().w(px(1.)).bg(theme.border))
                    .child(approval_mode)
                    .child(div().flex_1())
                    .when_some(context, |this, context| {
                        let can_compact = provider == Provider::Claude && !busy;
                        this.child(context_meter(
                            self.state.clone(),
                            context,
                            can_compact,
                            &theme,
                        ))
                    })
                    .children(chip_element)
                    .child(action),
            )
            .test_support()
            .into_any_element()
    }
}

fn provider_icon(provider: Provider) -> impl IntoElement {
    match provider {
        Provider::Claude => Icon::new(Logo::Claude)
            .size(px(14.))
            .text_color(gpui::rgb(0xd97757)),
        Provider::Codex => Icon::new(Logo::OpenAi).size(px(14.)),
    }
}

/// ProviderPicker.svelte: the agent, and a menu above it to switch.
fn provider_picker(state: Entity<AppState>, provider: Provider, theme: &Theme) -> impl IntoElement {
    let trigger_theme = theme.clone();
    let menu_theme = theme.clone();
    Popover::new("provider-picker")
        .anchor(Anchor::BottomLeft)
        .offset(px(6.))
        .trigger_with(move |open, _, _| {
            let theme = trigger_theme;
            div()
                .id("provider")
                .flex()
                .items_center()
                .gap_1p5()
                .h_7()
                .px_2()
                .rounded(theme.radius.md)
                .text_token(TextSize::Sm)
                .cursor_pointer()
                .map(|this| {
                    if open {
                        this.bg(theme.accent).text_color(theme.foreground)
                    } else {
                        this.text_color(theme.muted_foreground)
                    }
                })
                .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
                .child(provider_icon(provider))
                .child(provider_label(provider))
                .child(Icon::new(IconName::ChevronDown).size(px(12.)).opacity(0.7))
                .test_support()
                .into_any_element()
        })
        .content(move |_, _, cx| {
            let theme = menu_theme;
            let options = [Provider::Claude, Provider::Codex].map(|value| {
                let state = state.clone();
                div()
                    .id(match value {
                        Provider::Claude => "provider-claude",
                        Provider::Codex => "provider-codex",
                    })
                    .flex()
                    .items_center()
                    .gap_2()
                    .h_8()
                    .w_full()
                    .px_2()
                    .rounded(theme.radius.md)
                    .text_token(TextSize::Sm)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.accent))
                    .on_click(cx.listener(move |popover, _, window, cx| {
                        popover.dismiss(window, cx);
                        if value != provider {
                            state
                                .read(cx)
                                .send(ClientMessage::SetProvider { provider: value });
                        }
                    }))
                    .child(provider_icon(value))
                    .child(div().flex_1().child(provider_label(value)))
                    .when(value == provider, |this| {
                        this.child(
                            Icon::new(IconName::Check)
                                .size(px(14.))
                                .text_color(theme.muted_foreground),
                        )
                    })
                    .test_support()
            });
            div()
                .w(px(176.))
                .p_1()
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.card)
                .shadow_lg()
                .children(options)
        })
}

/// ContextMeter.svelte: a ring and the percentage (or the tokens, before the window is known),
/// with a popup to compact.
fn context_meter(
    state: Entity<AppState>,
    context: ContextUsage,
    can_compact: bool,
    theme: &Theme,
) -> impl IntoElement {
    let percent = context
        .max_tokens
        .filter(|max| *max > 0)
        .map(|max| (((context.used_tokens as f64 / max as f64) * 100.).round() as u32).min(100));
    let label = match percent {
        Some(percent) => format!("{percent}%"),
        None => format_tokens(context.used_tokens),
    };
    let ring_color = match percent {
        Some(percent) if percent >= 80 => Some(theme.warning),
        _ => None,
    };
    let trigger_theme = theme.clone();
    let menu_theme = theme.clone();
    Popover::new("context-meter")
        .anchor(Anchor::BottomRight)
        .offset(px(6.))
        .trigger_with(move |_, _, _| {
            let theme = trigger_theme;
            div()
                .id("context")
                .flex()
                .items_center()
                .gap_1p5()
                .h_7()
                .px_2()
                .rounded(theme.radius.md)
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .cursor_pointer()
                .hover(|style| style.bg(theme.accent).text_color(theme.foreground))
                .child(ring(percent, ring_color))
                .child(label)
                .test_support()
                .into_any_element()
        })
        .content(move |_, _, cx| {
            let theme = menu_theme;
            let used = group_thousands(context.used_tokens);
            let heading = match context.max_tokens {
                Some(max) => format!("{used} of {} tokens in context", group_thousands(max)),
                None => format!("{used} tokens in context"),
            };
            div()
                .w(px(256.))
                .p_3()
                .rounded(theme.radius.lg)
                .border_1()
                .border_color(theme.border)
                .bg(theme.card)
                .shadow_lg()
                .text_token(TextSize::Xs)
                .child(
                    div()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(heading),
                )
                .child(
                    div().mt_1().text_color(theme.muted_foreground).child(
                        "Compacting replaces the chat so far with a summary, so later turns cost less and the chat keeps fitting.",
                    ),
                )
                .child(
                    div()
                        .id("compact-now")
                        .mt_2p5()
                        .flex()
                        .items_center()
                        .justify_center()
                        .h_7()
                        .w_full()
                        .rounded(theme.radius.md)
                        .border_1()
                        .border_color(theme.border)
                        .map(|this| {
                            if can_compact {
                                let state = state.clone();
                                this.cursor_pointer()
                                    .hover(|style| style.bg(theme.accent))
                                    .on_click(cx.listener(move |popover, _, window, cx| {
                                        popover.dismiss(window, cx);
                                        state.read(cx).send(ClientMessage::Compact);
                                    }))
                            } else {
                                this.opacity(0.5)
                            }
                        })
                        .child("Compact now")
                        .test_support(),
                )
        })
}

/// The meter's ring: a faint full circle and the used part from the top, clockwise (the SVG's
/// r = 7 in an 18 viewBox, 2.5 wide, drawn at 14px).
fn ring(percent: Option<u32>, color: Option<Hsla>) -> impl IntoElement {
    canvas(
        |_, _, _| {},
        move |bounds, _, window, _| {
            let text = window.text_style().color;
            let used = color.unwrap_or(text);
            let scale = bounds.size.width / px(18.);
            let center = bounds.center();
            let radius = px(7. * scale);
            let width = px(2.5 * scale);
            let at = |angle: f32| {
                point(
                    center.x + radius * angle.cos(),
                    center.y + radius * angle.sin(),
                )
            };
            let top = -PI / 2.;
            let arc = |from: f32, to: f32| {
                let mut path = PathBuilder::stroke(width);
                path.move_to(at(from));
                // Two halves, so a full circle is drawn too.
                let middle = (from + to) / 2.;
                for (end, large) in [(middle, false), (to, false)] {
                    path.arc_to(point(radius, radius), px(0.), large, true, at(end));
                }
                path.build()
            };
            if let Ok(path) = arc(top, top + 2. * PI) {
                window.paint_path(path, text.opacity(0.25));
            }
            if let Some(percent) = percent.filter(|percent| *percent > 0)
                && let Ok(path) = arc(top, top + 2. * PI * percent as f32 / 100.)
            {
                window.paint_path(path, used);
            }
        },
    )
    .size(px(14.))
    .flex_shrink_0()
}
