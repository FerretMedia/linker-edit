//! Dialog.svelte and the small controls the dialogs share: Segmented.svelte, Switch.svelte, the
//! buttons (`h-8 rounded-md bg-primary px-3 text-sm`, the outlined one, the text link) and the
//! text field (`h-8 w-full rounded-md border bg-transparent px-2.5 text-sm focus:border-primary`).
//!
//! The dialog is modal like a `<dialog>` opened with showModal(): a black/40 backdrop over the
//! window, a 30rem card in the middle; Escape and a click on the backdrop close it.

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, KeyDownEvent, ParentElement, Pixels, RenderOnce, SharedString,
    Stateful, StatefulInteractiveElement, Styled, Window, div, hsla, prelude::FluentBuilder, px,
};
use gpui_kit::TestSupportExt;
use gpui_kit::component::Icon;
use gpui_kit::component::input::{Input, InputState};

use crate::icons::IconName;
use crate::theme::{TextSize, TextStyled, Theme};

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
                                icon_button("dialog-close", IconName::X, theme)
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

/// The header's close button and others like it: `size-7 rounded-md text-muted-foreground hover:bg-accent`.
pub fn icon_button(id: impl Into<ElementId>, icon: IconName, theme: &Theme) -> Stateful<Div> {
    let (accent, foreground) = (theme.accent, theme.foreground);
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .size_7()
        .rounded(theme.radius.md)
        .text_color(theme.muted_foreground)
        .cursor_pointer()
        .hover(move |style| style.bg(accent).text_color(foreground))
        .child(Icon::new(icon).size(px(16.)))
}

/// `h-8 rounded-md bg-primary px-3 text-sm text-primary-foreground`, at half opacity when disabled.
pub fn primary_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    disabled: bool,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .h_8()
        .px_3()
        .rounded(theme.radius.md)
        .bg(theme.primary)
        .text_color(theme.primary_foreground)
        .text_token(TextSize::Sm)
        .map(|this| {
            if disabled {
                this.opacity(0.5)
            } else {
                this.cursor_pointer()
                    .hover(|style| style.opacity(0.9))
                    .on_click(on_click)
            }
        })
        .child(label.into())
}

/// `h-8 rounded-md border px-3 text-sm hover:bg-accent`.
pub fn outline_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    theme: &Theme,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let accent = theme.accent;
    div()
        .id(id)
        .flex()
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .gap_1p5()
        .h_8()
        .px_3()
        .rounded(theme.radius.md)
        .border_1()
        .border_color(theme.border)
        .text_token(TextSize::Sm)
        .cursor_pointer()
        .hover(move |style| style.bg(accent))
        .on_click(on_click)
        .child(label.into())
}

/// A small link-like button: `text-2xs text-primary hover:underline`.
pub fn text_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    color: gpui::Hsla,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .text_token(TextSize::X2s)
        .text_color(color)
        .cursor_pointer()
        .hover(|style| style.underline())
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

/// Segmented.svelte: a small choice of a few options, all in view.
pub fn segmented<T: Copy + PartialEq + 'static>(
    id: &'static str,
    options: &[(T, &'static str)],
    value: T,
    theme: &Theme,
    on_change: impl Fn(T, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    let on_change = std::rc::Rc::new(on_change);
    div()
        .id(id)
        .flex()
        .flex_row()
        .flex_none()
        .self_start()
        .rounded(theme.radius.lg)
        .border_1()
        .border_color(theme.border)
        .bg(theme.muted)
        .p_0p5()
        .children(options.iter().enumerate().map(|(index, (option, label))| {
            let selected = *option == value;
            let option = *option;
            let on_change = on_change.clone();
            let foreground = theme.foreground;
            div()
                .id((id, index))
                .flex()
                .items_center()
                .h_7()
                .px_3()
                .rounded(theme.radius.md)
                .text_token(TextSize::Xs)
                .cursor_pointer()
                .map(|this| {
                    if selected {
                        this.bg(theme.card)
                            .text_color(theme.foreground)
                            .shadow_sm()
                            .border_1()
                            .border_color(theme.border)
                    } else {
                        this.text_color(theme.muted_foreground)
                            .hover(move |style| style.text_color(foreground))
                            .on_click(move |_, window, cx| on_change(option, window, cx))
                    }
                })
                .child(*label)
                .test_support()
        }))
}

/// Switch.svelte: `h-5 w-9 rounded-full`, primary when on, with a white knob.
pub fn switch(
    id: impl Into<ElementId>,
    checked: bool,
    theme: &Theme,
    on_change: impl Fn(bool, &mut Window, &mut App) + 'static,
) -> Stateful<Div> {
    div()
        .id(id)
        .relative()
        .flex()
        .flex_shrink_0()
        .items_center()
        .h_5()
        .w_9()
        .rounded_full()
        .bg(if checked { theme.primary } else { theme.input })
        .cursor_pointer()
        .on_click(move |_, window, cx| on_change(!checked, window, cx))
        .child(
            div()
                .absolute()
                .left(if checked { px(18.) } else { px(2.) })
                .size_4()
                .rounded_full()
                .bg(gpui::white())
                .shadow_sm(),
        )
}

/// A muted caption: `text-xs text-muted-foreground`.
pub fn caption(text: impl Into<SharedString>, theme: &Theme) -> Div {
    div()
        .text_token(TextSize::Xs)
        .text_color(theme.muted_foreground)
        .child(text.into())
}
