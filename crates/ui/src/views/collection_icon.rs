//! CollectionIcon.svelte, a workspace's or series' icon on its color (a lucide icon, an emoji or
//! its letters), and CollectionIconPicker.svelte, which picks one: letters, a lucide icon or an
//! emoji, and a color. The picker is controlled: it shows the icon it is given and emits the
//! changed one, which its owner sends to the server.

use gpui::{
    AppContext, Context, Entity, EventEmitter, Focusable, FontWeight, InteractiveElement,
    IntoElement, ParentElement, Pixels, Render, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{InputEvent, InputState};
use linker_edit_protocol::{CollectionIcon as Icon_, IconColor};

use crate::collection_icons::{
    EMOJI_SUGGESTIONS, ICON_COLORS, LUCIDE_ICONS, color_name, glyph, icon_color, icon_text,
    lucide_icon, monogram, swatch, tile,
};
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::dialog::bare_input;
use crate::views::dialog::segmented;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconSize {
    Sm,
    Md,
    Lg,
}

/// CollectionIcon.svelte.
#[derive(IntoElement)]
pub struct CollectionIcon {
    icon: Icon_,
    name: SharedString,
    size: IconSize,
}

impl CollectionIcon {
    pub fn new(icon: &Icon_, name: impl Into<SharedString>, size: IconSize) -> Self {
        CollectionIcon {
            icon: icon.clone(),
            name: name.into(),
            size,
        }
    }
}

impl RenderOnce for CollectionIcon {
    fn render(self, _: &mut Window, cx: &mut gpui::App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let color = icon_color(&self.icon);
        let (box_size, radius, text, glyph_size): (Pixels, Pixels, TextSize, Pixels) =
            match self.size {
                IconSize::Sm => (px(20.), theme.radius.sm, TextSize::X2s, px(12.)),
                IconSize::Md => (px(24.), theme.radius.md, TextSize::Xs, px(14.)),
                IconSize::Lg => (px(36.), theme.radius.lg, TextSize::Sm, px(20.)),
            };
        let text_value = icon_text(&self.icon, &self.name);
        div()
            .flex()
            .flex_shrink_0()
            .items_center()
            .justify_center()
            .size(box_size)
            .rounded(radius)
            .bg(tile(color))
            .text_color(glyph(color, theme))
            .text_token(text)
            .font_weight(FontWeight::SEMIBOLD)
            .map(|this| match &self.icon {
                Icon_::Lucide { name, .. } => {
                    this.child(Icon::new(lucide_icon(name)).size(glyph_size))
                }
                Icon_::Emoji { emoji, .. } => this.child(emoji.clone()),
                Icon_::Monogram { .. } => this.child(text_value),
            })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Monogram,
    Lucide,
    Emoji,
}

pub struct IconChanged(pub Icon_);

/// CollectionIconPicker.svelte.
pub struct CollectionIconPicker {
    icon: Icon_,
    name: String,
    letters: Entity<InputState>,
    emoji: Entity<InputState>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<IconChanged> for CollectionIconPicker {}

fn color_of(icon: &Icon_) -> IconColor {
    icon_color(icon)
}

impl CollectionIconPicker {
    pub fn new(icon: Icon_, name: &str, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let text = match &icon {
            Icon_::Monogram { text, .. } => text.clone(),
            _ => String::new(),
        };
        let emoji = match &icon {
            Icon_::Emoji { emoji, .. } => emoji.clone(),
            _ => String::new(),
        };
        let letters = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(monogram(name))
                .default_value(text)
        });
        let emoji = cx.new(|cx| InputState::new(window, cx).default_value(emoji));
        let subscriptions = vec![
            cx.subscribe(&letters, |this, input, event: &InputEvent, cx| {
                if let (InputEvent::Change, Icon_::Monogram { color, .. }) = (event, &this.icon) {
                    // maxlength="3"
                    let text: String = input.read(cx).value().chars().take(3).collect();
                    let next = Icon_::Monogram {
                        text,
                        color: *color,
                    };
                    this.change(next, cx);
                }
            }),
            cx.subscribe(&emoji, |this, input, event: &InputEvent, cx| {
                let value = input.read(cx).value().trim().to_string();
                if let (InputEvent::Change, Icon_::Emoji { color, .. }) = (event, &this.icon)
                    && !value.is_empty()
                {
                    let next = Icon_::Emoji {
                        emoji: value.chars().take(16).collect(),
                        color: *color,
                    };
                    this.change(next, cx);
                }
            }),
        ];
        CollectionIconPicker {
            icon,
            name: name.into(),
            letters,
            emoji,
            _subscriptions: subscriptions,
        }
    }

    pub fn icon(&self) -> &Icon_ {
        &self.icon
    }

    /// The icon as the server has it now.
    pub fn set_icon(&mut self, icon: Icon_, cx: &mut Context<Self>) {
        self.icon = icon;
        cx.notify();
    }

    fn change(&mut self, icon: Icon_, cx: &mut Context<Self>) {
        self.icon = icon.clone();
        cx.emit(IconChanged(icon));
        cx.notify();
    }

    fn pick_kind(&mut self, kind: Kind, window: &mut Window, cx: &mut Context<Self>) {
        let color = color_of(&self.icon);
        let next = match kind {
            Kind::Monogram => {
                self.letters
                    .update(cx, |input, cx| input.set_value("", window, cx));
                Icon_::Monogram {
                    text: String::new(),
                    color,
                }
            }
            Kind::Lucide => Icon_::Lucide {
                name: "folder".into(),
                color,
            },
            Kind::Emoji => {
                let emoji = EMOJI_SUGGESTIONS[0].to_string();
                self.emoji
                    .update(cx, |input, cx| input.set_value(emoji.clone(), window, cx));
                Icon_::Emoji { emoji, color }
            }
        };
        self.change(next, cx);
    }
}

impl Render for CollectionIconPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::of(cx).clone();
        let kind = match self.icon {
            Icon_::Monogram { .. } => Kind::Monogram,
            Icon_::Lucide { .. } => Kind::Lucide,
            Icon_::Emoji { .. } => Kind::Emoji,
        };
        let color = color_of(&self.icon);
        let entity = cx.entity();
        let accent = theme.accent;
        let small_field = |input: &Entity<InputState>, width: Pixels, center: bool| {
            let focused = input.read(cx).focus_handle(cx).is_focused(window);
            div()
                .flex()
                .items_center()
                .h_8()
                .w(width)
                .px_2p5()
                .rounded(theme.radius.md)
                .border_1()
                .border_color(if focused { theme.primary } else { theme.border })
                .text_token(TextSize::Sm)
                .when(center, |this| this.justify_center())
                .child(bare_input(input, TextSize::Sm))
        };
        let body = match &self.icon {
            Icon_::Monogram { .. } => div()
                .flex()
                .items_center()
                .gap_2()
                .text_token(TextSize::Xs)
                .text_color(theme.muted_foreground)
                .child("Letters")
                .child(small_field(&self.letters, px(96.), false).id("collection-letters"))
                .into_any_element(),
            Icon_::Lucide { name, .. } => div()
                .flex()
                .flex_wrap()
                .gap_0p5()
                .children(LUCIDE_ICONS.iter().map(|(lucide, glyph_icon)| {
                    let selected = name == lucide;
                    let picked = lucide.to_string();
                    div()
                        .id(SharedString::from(format!("collection-icon-{lucide}")))
                        .flex()
                        .items_center()
                        .justify_center()
                        .size_8()
                        .rounded(theme.radius.md)
                        .cursor_pointer()
                        .hover(move |style| style.bg(accent))
                        .map(|this| {
                            if selected {
                                this.bg(theme.accent)
                                    .text_color(theme.foreground)
                                    .border_1()
                                    .border_color(theme.border)
                            } else {
                                this.text_color(theme.muted_foreground)
                            }
                        })
                        .tooltip(move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(*lucide).build(window, cx)
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.change(
                                Icon_::Lucide {
                                    name: picked.clone(),
                                    color,
                                },
                                cx,
                            )
                        }))
                        .child(Icon::new(*glyph_icon).size(px(16.)))
                        .test_support()
                }))
                .into_any_element(),
            Icon_::Emoji { emoji, .. } => div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_0p5()
                .children(
                    EMOJI_SUGGESTIONS
                        .iter()
                        .enumerate()
                        .map(|(index, suggestion)| {
                            let selected = emoji == suggestion;
                            div()
                                .id(("collection-emoji", index))
                                .flex()
                                .items_center()
                                .justify_center()
                                .size_8()
                                .rounded(theme.radius.md)
                                .text_token(TextSize::Base)
                                .cursor_pointer()
                                .hover(move |style| style.bg(accent))
                                .when(selected, |this| {
                                    this.bg(theme.accent).border_1().border_color(theme.border)
                                })
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.emoji.update(cx, |input, cx| {
                                        input.set_value(suggestion.to_string(), window, cx)
                                    });
                                    this.change(
                                        Icon_::Emoji {
                                            emoji: suggestion.to_string(),
                                            color,
                                        },
                                        cx,
                                    )
                                }))
                                .child(*suggestion)
                                .test_support()
                        }),
                )
                .child(div().ml_1().child(small_field(&self.emoji, px(64.), true)))
                .into_any_element(),
        };
        let swatches = ICON_COLORS.iter().map(|option| {
            let option = *option;
            let selected = option == color;
            let mut next = self.icon.clone();
            match &mut next {
                Icon_::Lucide { color, .. }
                | Icon_::Emoji { color, .. }
                | Icon_::Monogram { color, .. } => *color = option,
            }
            div()
                .id(SharedString::from(format!(
                    "collection-color-{}",
                    color_name(option)
                )))
                .relative()
                .size_5()
                .rounded_full()
                .bg(swatch(option))
                .cursor_pointer()
                .tooltip(move |window, cx| {
                    gpui_kit::component::tooltip::Tooltip::new(color_name(option)).build(window, cx)
                })
                .on_click(cx.listener(move |this, _, _, cx| this.change(next.clone(), cx)))
                .when(selected, |this| {
                    this.child(
                        div()
                            .absolute()
                            .top(px(-4.))
                            .left(px(-4.))
                            .size(px(28.))
                            .rounded_full()
                            .border_2()
                            .border_color(theme.foreground),
                    )
                })
                .test_support()
        });
        let _ = &self.name;
        div()
            .flex()
            .flex_col()
            .gap_3()
            .child(segmented(
                "collection-icon-type",
                &[
                    (Kind::Monogram, "Letters"),
                    (Kind::Lucide, "Icon"),
                    (Kind::Emoji, "Emoji"),
                ],
                kind,
                &theme,
                move |kind, window, cx| {
                    entity.update(cx, |this, cx| this.pick_kind(kind, window, cx))
                },
            ))
            .child(body)
            .child(div().flex().items_center().gap_1p5().children(swatches))
    }
}
