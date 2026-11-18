//! Dialog.svelte and what the dialogs share: their buttons (the pages' at `text-sm`) and the text
//! field (`h-8 w-full rounded-md border bg-transparent px-2.5 text-sm focus:border-primary`).
//! Switch.svelte and Segmented.svelte are [`settings_parts`]'s.
//!
//! The dialog is modal like a `<dialog>` opened with showModal(): a black/40 backdrop over the
//! window, a 30rem card in the middle; Escape and a click on the backdrop close it.

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, RenderOnce, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, div, hsla, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::input::{Input, InputState};

use crate::icons::IconName;
use crate::theme::{TextSize, TextStyled, Theme};
use crate::views::settings_parts::{self, disable, icon_button};

pub type Handler = Box<dyn Fn(&mut Window, &mut App) + 'static>;

/// The dialog's frame: backdrop, card, header with its title and the close button, and the body.
#[derive(IntoElement)]
pub struct Dialog {
    id: ElementId,
    title: SharedString,
    focus: FocusHandle,
    width: Pixels,
    on_close: Handler,
    body: Vec<AnyElement>,
}

impl Dialog {
    pub fn new(
        id: impl Into<ElementId>,
        title: impl Into<SharedString>,
        focus: &FocusHandle,
        on_close: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        Dialog {
            id: id.into(),
            title: title.into(),
            focus: focus.clone(),
            width: px(480.),
            on_close: Box::new(on_close),
            body: Vec::new(),
        }
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl RenderOnce for Dialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = Theme::of(cx);
        let close = std::rc::Rc::new(self.on_close);
        let (on_escape, on_backdrop, on_button) = (close.clone(), close.clone(), close);
        let viewport = window.viewport_size();
        div()
            .id(self.id.clone())
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .bg(hsla(0., 0., 0., 0.4))
            .occlude()
            .track_focus(&self.focus)
            .on_key_down(move |event: &KeyDownEvent, window, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    on_escape(window, cx);
                }
            })
            .on_click(move |_, window, cx| on_backdrop(window, cx))
            .child(
                div()
                    .id("dialog-card")
                    .w(self.width.min(viewport.width - px(32.)))
                    .max_h(viewport.height - px(32.))
                    .flex()
                    .flex_col()
                    .rounded(theme.radius.xl)
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.card)
                    .text_color(theme.foreground)
                    .shadow_xl()
                    // A click in the card isn't one on the backdrop.
                    .on_click(|_, _, cx| cx.stop_propagation())
                    .child(
                        div()
                            .flex()
                            .flex_shrink_0()
                            .items_center()
                            .gap_2()
                            .h_12()
                            .px_4()
                            .border_b_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .text_token(TextSize::Sm)
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(self.title),
                            )
                            .child(
                                icon_button("dialog-close", IconName::X, px(16.), theme)
                                    .ml_auto()
                                    .on_click(move |_, window, cx| on_button(window, cx)),
                            ),
                    )
                    .child(
                        div()
                            .id("dialog-body")
                            .flex()
                            .flex_col()
                            .gap_4()
                            .p_4()
                            .min_h_0()
                            .overflow_y_scroll()
                            .children(self.body),
                    ),
            )
            .test_support()
    }
}

/// `h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground`, at half opacity when disabled:
/// the pages' primary button at the dialogs' text size.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    disabled: bool,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    disable(settings_parts::primary_button(id, theme), disabled)
        .justify_center()
        .text_token(TextSize::Sm)
        .when(!disabled, |this| this.on_click(on_click))
        .child(label.into())
}

/// `h-8 rounded-md border px-3 text-sm hover:bg-accent`.
pub fn outline_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    settings_parts::outline_button(id, theme)
        .justify_center()
        .text_token(TextSize::Sm)
        .on_click(on_click)
        .child(label.into())
}

/// The text field around an input: border, primary when focused.
pub fn field(input: &Entity<InputState>, window: &Window, cx: &App) -> Div {
    let theme = Theme::of(cx);
    let focused = input.read(cx).focus_handle(cx).is_focused(window);
    div()
        .flex()
        .items_center()
        .h_8()
        .w_full()
        .min_w_0()
        .px_2p5()
        .rounded(theme.radius.md)
        .border_1()
        .border_color(if focused { theme.primary } else { theme.border })
        .text_token(TextSize::Sm)
        .child(bare_input(input, TextSize::Sm))
}

/// An input with none of gpui-component's own frame, padding or size: the element around it
/// draws the field, as the Svelte `<input class="bg-transparent p-0 …">` inside its box.
pub fn bare_input(input: &Entity<InputState>, size: TextSize) -> Input {
    // Input's own `h` is for a multi-line input; Styled's sets the frame's height.
    Styled::h(
        Input::new(input).appearance(false).w_full(),
        size.line_height(),
    )
    .px_0()
    .py_0()
    .text_size(size.size())
    .line_height(size.line_height())
}

/// A muted caption: `text-xs text-muted-foreground`.
pub fn caption(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_token(TextSize::Xs)
        .text_color(theme.muted_foreground)
        .child(text.into())
}
