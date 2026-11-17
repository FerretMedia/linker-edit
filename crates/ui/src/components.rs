//! Small elements several views share. Each mirrors a recurring Tailwind class list in the
//! Svelte UI; the comment names it, so the two stay comparable.

use gpui::{
    App, ClickEvent, ElementId, FontWeight, InteractiveElement, IntoElement, ParentElement, Pixels,
    RenderOnce, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px, rgb,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;

use crate::icons::{IconName, Logo};
use crate::theme::{TextSize, TextStyled, Theme};

/// Linker's mark (LinkerLogo.svelte): the cards in the text color over a blue glow offset to
/// the bottom right and a pink one to the top left. GPUI tints an SVG with one color, so the
/// three layers are three SVGs stacked.
#[derive(IntoElement)]
pub struct LinkerLogo {
    size: Pixels,
}

impl LinkerLogo {
    pub fn new(size: Pixels) -> Self {
        LinkerLogo { size }
    }
}

impl RenderOnce for LinkerLogo {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let layer = |logo: Logo, color: gpui::Hsla| {
            div()
                .absolute()
                .inset_0()
                .child(Icon::new(logo).size(self.size).text_color(color))
        };
        div()
            .relative()
            .flex_shrink_0()
            .size(self.size)
            .child(layer(Logo::LinkerMarkBlue, rgb(0x4545e6).into()))
            .child(layer(Logo::LinkerMarkPink, rgb(0xa01d90).into()))
            .child(layer(Logo::LinkerMark, theme.foreground))
    }
}

/// A click handler a `RenderOnce` component keeps until it renders.
pub type OnClick = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

/// The header's toggles: `flex h-7 items-center gap-1.5 rounded-md border px-2 text-xs`,
/// muted until hovered or pressed.
#[derive(IntoElement)]
pub struct HeaderButton {
    id: ElementId,
    icon: IconName,
    label: SharedString,
    pressed: bool,
    on_click: Option<OnClick>,
}

impl HeaderButton {
    pub fn new(id: impl Into<ElementId>, icon: IconName, label: impl Into<SharedString>) -> Self {
        HeaderButton {
            id: id.into(),
            icon,
            label: label.into(),
            pressed: false,
            on_click: None,
        }
    }

    pub fn pressed(mut self, pressed: bool) -> Self {
        self.pressed = pressed;
        self
    }

    pub fn on_click(mut self, f: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_click = Some(Box::new(f));
        self
    }
}

impl RenderOnce for HeaderButton {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let (accent, foreground) = (theme.accent, theme.foreground);
        div()
            .id(self.id)
            .flex()
            .items_center()
            .gap_1p5()
            .h_7()
            .px_2()
            .rounded(theme.radius.md)
            .border_1()
            .border_color(theme.border)
            .text_token(TextSize::Xs)
            .cursor_pointer()
            .map(|this| {
                if self.pressed {
                    this.bg(accent).text_color(foreground)
                } else {
                    this.text_color(theme.muted_foreground)
                        .hover(move |style| style.bg(accent).text_color(foreground))
                }
            })
            .child(Icon::new(self.icon).size(px(14.)))
            .child(self.label)
            .when_some(self.on_click, |this, f| this.on_click(f))
            .test_support()
    }
}

/// A placeholder block for what a later view fills in: its name, muted, centered.
pub fn placeholder(label: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    let theme = Theme::of(cx);
    div()
        .flex()
        .flex_1()
        .items_center()
        .justify_center()
        .text_token(TextSize::Xs)
        .text_color(theme.muted_foreground.opacity(0.7))
        .font_weight(FontWeight::NORMAL)
        .child(label.into())
}
